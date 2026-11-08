// SPDX-License-Identifier: Apache-2.0
//! Exercise the production permission wait against real Application and Core state.
use super::*;
use nola_core::agent::{AgentCoreConfig, AgentSubmission, SessionId};
#[path = "../../core/nola-application/tests/support/mod.rs"]
mod source_support;

struct PermissionWaitFixture {
    _root: tempfile::TempDir,
    environment: Vec<(&'static str, Option<std::ffi::OsString>)>,
    task_id: String,
    cancellation: RuntimeExecutionControl,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for PermissionWaitFixture {
    fn drop(&mut self) {
        self.cancellation.request();
        wake_task_waiters();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        task_handles().lock().unwrap().remove(&self.task_id);
        if let Ok(lease) = crate::runtime_task_port::runtime_task_lease_state(&self.task_id) {
            crate::runtime_task_port::release_runtime_task_handle(&lease);
        }
        for (key, value) in &self.environment {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn wait_for_fact<T>(stage: &str, mut read: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(value) = read() {
            return value;
        }
        assert!(Instant::now() < deadline, "permission wait fixture timed out at {stage}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn production_permission_wait_requires_exact_store_grant() {
    exercise_permission_parent_handoff(0);
}

#[test]
fn host_similarity_grant_cannot_override_a_new_core_permission_wait() {
    exercise_permission_parent_handoff(20);
}

#[test]
fn stopping_background_permission_wait_preserves_absence_of_user_decision() {
    exercise_permission_parent_handoff(21);
}

#[test]
fn core_settlement_closes_wait_without_replaying_a_local_decision() {
    exercise_permission_parent_handoff(22);
}

#[test]
fn cold_permission_resume_reuses_exact_original_decision_without_parent_handoff() {
    exercise_permission_parent_handoff(1);
}

#[test]
fn cold_permission_successor_recovers_without_parent_lease_projection() {
    exercise_permission_parent_handoff(2);
}

#[test]
fn cold_permission_successor_survives_parent_adapter_reopen() {
    exercise_permission_parent_handoff(3);
}

#[test]
fn cold_permission_successor_rejects_stale_predecessor_handle() {
    exercise_permission_parent_handoff(4);
}

#[test]
fn bound_terminal_worker_recovery_never_restarts_model() {
    exercise_permission_parent_handoff(5);
}

#[test]
fn damaged_agent_budget_cannot_dispatch_or_replace_the_bound_run() {
    exercise_permission_parent_handoff(10);
}

#[test]
fn exhausted_task_cannot_start_effect_or_dispatch_bound_agent_work() {
    exercise_permission_parent_handoff(11);
}

#[test]
fn waiting_permission_budget_is_observed_and_sealed_without_a_worker_claim() {
    exercise_permission_parent_handoff(12);
}

#[test]
fn permission_closure_preserves_prior_approval_without_reviving_execution() {
    exercise_permission_parent_handoff(13);
}

#[test]
fn standalone_permission_without_suspension_closes_from_its_original_run() {
    exercise_permission_parent_handoff(14);
    exercise_permission_parent_handoff(15);
}

#[test]
fn standalone_denied_permission_closes_on_model_completion_and_failure() {
    exercise_permission_parent_handoff(16);
    exercise_permission_parent_handoff(17);
}

#[test]
fn standalone_approved_permission_revokes_grant_after_tool_result_and_terminal() {
    exercise_permission_parent_handoff(18);
    exercise_permission_parent_handoff(19);
}

#[test]
fn terminal_replay_without_receipt_cannot_borrow_parent_authority() {
    exercise_permission_parent_handoff(6);
}

#[test]
fn permission_suspension_cancellation_retains_terminal_execution_evidence() {
    exercise_permission_parent_handoff(7);
}

#[test]
fn guidance_without_local_worker_is_durable_and_queue_overflow_is_explicit() {
    exercise_permission_parent_handoff(8);
}

#[test]
fn resumed_host_adopts_only_explicit_exact_worker_authority_and_keeps_frozen_settings() {
    exercise_permission_parent_handoff(9);
}

#[test]
fn bound_terminal_worker_child() {
    let Ok(encoded) = std::env::var("NOLA_TEST_BOUND_TERMINAL_ATTEMPT") else { return; };
    let attempt: nola_application::RuntimeTaskAttempt = serde_json::from_str(&encoded).unwrap();
    let lease = crate::runtime_task_port::register_runtime_task_attempt(attempt.clone());
    let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
    config.session.session_id = Some(std::env::var("NOLA_TEST_BOUND_TERMINAL_CHILD").unwrap());
    // No provider/model fixture exists: reaching model execution is a failure.
    let owner = std::env::var("NOLA_TEST_BOUND_TERMINAL_OWNER").unwrap();
    assert_eq!(run_agent_task(
        &attempt.task_id, "terminal recovery", "must never execute again", &owner,
        7, attempt.worker_id.clone(), lease, 60_000, config,
        RuntimeExecutionControl::default(), None,
    ), 0);
}

fn exercise_permission_parent_handoff(mode: usize) {
    let cold = mode > 0 && !matches!(mode, 20..=22);
    let real_permission = matches!(mode, 0..=4 | 7 | 12..=22);
    use nola_core::agent::host::*;
    let _environment = crate::store::test_env_lock();
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("permission.sqlite");
    // The real producer has fixed identities; each scenario owns an isolated
    // Store and the existing environment lock, never a shared execution.
    let task_id = "task".to_string();
    let parent_id = "parent".to_string();
    let child_id = "child".to_string();
    let mut fixture = PermissionWaitFixture {
        environment: ["NOLA_HOME", "NOLA_DB_PATH", "NOLA_AGENT_TASK_LEASE_MS"].into_iter()
            .map(|key| (key, std::env::var_os(key))).collect(),
        task_id: task_id.clone(), cancellation: RuntimeExecutionControl::default(),
        worker: None, _root: root,
    };
    std::env::set_var("NOLA_HOME", fixture._root.path());
    std::env::set_var("NOLA_DB_PATH", &database);
    std::env::set_var("NOLA_AGENT_TASK_LEASE_MS", "1000");
    let now = unix_epoch_millis();
    let mut source = source_support::open_runtime(&database).unwrap();
    source_support::agent_task_admission::draft(&mut source,None);
    let admission=source_support::agent_task_admission::source_fact(&mut source);
    source.execute(nola_application::ApplicationCommand::ImportHostAgentTaskAdmission {admission:admission.clone()}).unwrap();
    let parent_turn=admission.parent_turn_id.clone();
    let mut store=nola_store::NolaStore::open(&database).unwrap();
    let original=store.claim_runtime_task(&task_id,"fixture-worker","fixture-generation",
        "fixture-original-token",now,now+60_000).unwrap().unwrap();
    let lease=crate::runtime_task_port::register_runtime_task_attempt(original.clone());
    let authority=crate::runtime_task_port::RuntimeTaskExecutionAuthority::new(lease.clone());
    drop(source);
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(), title: "Fixture model".into(), route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available, exposure: HostCapabilityExposure::Internal,
        permission_domain: None, input_schema: json!({"type":"object"}), input_normalization: Default::default(),
        output_policy: HostOutputPolicy { preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true },
        supports_streaming: true, supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial, cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    let mut application = crate::application_port::CliApplicationPort::persistent_default().unwrap();
    let mut capabilities = [("model.stream".into(), capability.clone())]
        .into_iter().collect::<std::collections::BTreeMap<_, _>>();
    if real_permission {
        let mut write = capability;
        write.id = "file.write".into();
        write.title = "Write fixture file".into();
        write.route = HostCapabilityRoute::Tool;
        write.exposure = HostCapabilityExposure::Visible;
        write.permission_domain = Some("file.write".into());
        write.supports_streaming = false;
        capabilities.insert(write.id.clone(), write);
    }
    let mut child_identity = crate::test_model_identity::identity();
    if matches!(mode, 1..=4 | 7 | 11..=19 | 21) {
        let mut invocation = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
        child_identity.host_invocation = Some(crate::core_agent_runtime::fixture_host_invocation_binding(&mut invocation).unwrap());
    }
    let frontier = application.load_conversation(&child_id).unwrap();
    let child_run = application.begin_task_assistant_turn(
        nola_application::RuntimeTaskLeaseWitness::from(&original), nola_application::BeginAssistantTurn {
        expected_configuration_revision: frontier.as_ref().map(|conversation| conversation.configuration_revision),
        expected_parent_message_id: frontier.as_ref().and_then(|conversation| conversation.messages.last().map(|message| message.id.clone())),
        conversation_id: child_id.clone(), conversation_title: "Permission child".into(),
        user_message_id: "fixture-user".into(), user_part_id: "fixture-user-part".into(),
        user_text: "Wait for permission".into(), user_content_json: None,
        assistant_message_id: "fixture-assistant".into(), context_checkpoint_id: None,
        core_config: AgentCoreConfig {
            turn_identity: child_identity,
            tools: if !real_permission { vec![] } else { vec![json!({"type":"function","function":{
                "name":"file.write","description":"Write the fixture file",
                "parameters":{"type":"object"}
            }})] },
            session_id: Some(SessionId::from_string(child_id.clone())), model: Some("fixture-model".into()),
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("fixture-host".into()), revision: 1, availability_epoch: 1, lease: None,
                capabilities: capabilities.into_iter().collect(),
            }), ..Default::default()
        },
        submission: AgentSubmission::UserInput {
            items: vec![nola_core::agent::AgentItem::user_message(
                nola_core::agent::ItemId::from_string("fixture-user".into()), "Wait for permission")],
            turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("fixture-child-start".into()), submitted_at_epoch_ms: Some(now),
            delivery: nola_core::agent::AgentInputDelivery::CurrentTurn,
        }, created_at_ms: now,
    }).unwrap();
    // The hot path must suspend an actual Core permission effect, not a
    // fabricated ID while the Run is still waiting for model output.
    let core_permission = if real_permission {
        let claimed = application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: child_run.begin.run_id.clone(), worker_generation: "permission-model".into(),
            lease_owner: "fixture".into(), claimed_at_ms: now + 1,
            lease_expires_at_ms: now + 60_000, limit: 16,
        }).unwrap();
        let model = claimed.requests.into_iter()
            .find(|request| matches!(request.payload, HostRequest::ModelStream { .. })).unwrap();
        for (index, payload) in [
            HostResponse::ExecutionStarted,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(), model: "fixture-model".into(), reason: "initial".into(),
            },
            HostResponse::ModelCompleted {
                items: vec![], tool_calls: vec![json!({"call_id":"fixture-write","name":"file.write",
                    "arguments":{"path":"fixture-output.txt","content":"test"}})], usage: json!({}),
            },
        ].into_iter().enumerate() {
            application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                conversation_id: child_id.clone(), run_id: child_run.begin.run_id.clone(),
                invocation_config_digest: child_run.invocation_config_digest.clone(),
                received_at_ms: now + 2 + index as i64,
                response: HostResponseEnvelope {
                    attempt_id: model.attempt_id.clone(), delegated_invocation_id: model.delegated_invocation_id.clone(),
                    request_id: model.request_id.clone(), effect_id: model.effect_id.clone(),
                    host_id: model.host_id.clone(), lease_id: model.lease_id.clone(),
                    session_id: model.session_id.clone(), turn_id: model.turn_id.clone(),
                    catalog_revision: model.catalog_revision, availability_epoch: model.availability_epoch,
                    delivery: model.delivery.clone(), sequence: index as u64 + 1, payload,
                },
            }).unwrap();
        }
        let requests = application.load_committed_host_requests(&child_run.begin.run_id).unwrap();
        assert!(!requests.requests.iter().any(|request| matches!(request.payload, HostRequest::ExecuteTool { .. })),
            "Core must not dispatch the write before authorization");
        Some(requests.requests.into_iter().find(|request|
            matches!(request.payload, HostRequest::RequestPermission { .. }))
            .expect("Core must emit the permission occurrence for the actual tool call"))
    } else { None };
    let emitted_permission_id = core_permission.as_ref().map(|request| {
        let HostRequest::RequestPermission { permission_request_id, .. } = &request.payload else { unreachable!() };
        permission_request_id.clone()
    });
    let original_core_permission_id = emitted_permission_id.clone().unwrap_or_default();
    if let Some(request) = core_permission.as_ref() {
        let HostRequest::RequestPermission { permission_request_id, capability_id, input,
            permission_instructions, .. } = &request.payload else { unreachable!() };
        let before = store.runtime_task(&task_id).unwrap().unwrap();
        let before_events = store.runtime_task_events(&task_id, 100).unwrap();
        for mismatch in ["run", "child", "watermark", "core", "capability", "input", "source"] {
            let mut continuation = json!({"child_session_id":child_id,"run_id":child_run.begin.run_id,
                "core_permission_request_id":permission_request_id,"capability":capability_id,
                "input":input,"input_watermark":7});
            let mut child = child_id.clone();
            let mut core_id = permission_request_id.clone();
            let mut capability = capability_id.clone();
            let mut arguments = input.clone();
            match mismatch {
                "run" => continuation["run_id"] = json!("another-run"),
                "child" => { child = "another-child".into(); continuation["child_session_id"] = json!(child); }
                "watermark" => continuation["input_watermark"] = json!(8),
                "core" => { core_id = "fabricated-core-request".into(); continuation["core_permission_request_id"] = json!(core_id); }
                "capability" => { capability = "file.read".into(); continuation["capability"] = json!(capability); }
                "input" => { arguments = json!({"path":"different-file"}); continuation["input"] = arguments.clone(); }
                _ => continuation["core_permission_source"] = json!({"forged":true}),
            }
            let error = store.suspend_runtime_task_for_permission(&nola_application::SuspendRuntimeTaskForPermission {
                task_id: task_id.clone(), attempt_id: original.id.clone(),
                expected_attempt_revision: original.revision, worker_generation: original.worker_generation.clone(),
                lease_token: original.lease_token.clone(), current_expiry_ms: original.lease_expires_at_ms,
                permission_request: nola_store_port::permission::StoredPermissionRequest {
                    closure: None,
                    request_id: "wrong-binding".into(), task_id: task_id.clone(), child_session_id: child,
                    capability, input_json: arguments.to_string(), scope_rule: "fixture-rule".into(),
                    created_at_ms: now, core_permission_request_id: Some(core_id),
                    permission_instructions_json: serde_json::to_string(permission_instructions).unwrap(),
                    status: "pending".into(), decision: None, custom_response: None, resolved_at_ms: None,
                },
                continuation_id: "wrong-continuation".into(), continuation_json: continuation.to_string(),
                progress_summary: "must not suspend".into(), last_activity: None, now_ms: now,
            }).unwrap_err();
            let expected = match mismatch {
                "run" | "child" | "watermark" => "canonical Task Run binding",
                "source" => "permission continuation identity mismatch",
                _ => "Core effect",
            };
            assert!(error.to_string().contains(expected), "{mismatch}: {error}");
            assert!(store.permission_request("wrong-binding").unwrap().is_none());
            assert!(store.latest_runtime_task_permission_suspension(&task_id).unwrap().is_none());
            assert_eq!(store.runtime_task(&task_id).unwrap().unwrap(), before);
            assert_eq!(store.runtime_task_events(&task_id, 100).unwrap(), before_events);
        }
    }
    if mode == 11 {
        let db = rusqlite::Connection::open(&database).unwrap();
        let effects = || db.prepare(
            "SELECT effect_id,request_id,status,payload_digest FROM effect_outbox WHERE run_id=?1 ORDER BY effect_id"
        ).unwrap().query_map([&child_run.begin.run_id], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?,
        ))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        let before_effects = effects();
        let delivery_count = || before_effects.iter().map(|effect| db.query_row(
            "SELECT COUNT(*) FROM effect_delivery_attempts WHERE effect_id=?1", [&effect.0],
            |row| row.get::<_, i64>(0)).unwrap()).sum::<i64>();
        let before_deliveries = delivery_count();
        assert!(!before_effects.is_empty());
        assert_eq!(before_deliveries, 0, "fixture model effect must never have been dispatched");
        let marker = fixture._root.path().join("forbidden-effect");
        let prior_revision = lease.snapshot().unwrap().revision;
        let refusal = check_task_effect_admission(&lease, Instant::now(), Some(Duration::ZERO))
            .and_then(|_| std::fs::write(&marker, b"unexpected effect").map_err(|e| e.to_string()));
        assert!(refusal.unwrap_err().contains("during preparation"));
        assert!(!marker.exists());
        assert!(lease.snapshot().unwrap().revision > prior_revision,
            "test must consume a real successful Store CAS before refusing effects");

        db.execute("UPDATE durable_tasks SET cancel_requested=1 WHERE id=?1", [&task_id]).unwrap();
        let revision_before_cancel = lease.snapshot().unwrap().revision;
        assert!(check_task_effect_admission(&lease, Instant::now(), None).is_err());
        assert_eq!(lease.snapshot().unwrap().revision, revision_before_cancel,
            "cancelled task must lose the Store CAS without advancing its lease witness");
        db.execute("UPDATE durable_tasks SET cancel_requested=0 WHERE id=?1", [&task_id]).unwrap();

        let application_store = NolaStore::open(&database).unwrap();
        let task = application_store.runtime_task(&task_id).unwrap().unwrap();
        let mut scheduling = task.scheduling().unwrap().unwrap();
        scheduling.resource_budget.max_wall_time_ms = Some(1);
        let mut expired = task.clone();
        expired.set_scheduling(scheduling).unwrap();
        db.execute("UPDATE durable_tasks SET metadata_json=?2,started_at_ms=?3 WHERE id=?1",
            rusqlite::params![task_id, expired.metadata_json, now - 1000]).unwrap();
        let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
        config.session.session_id = Some(child_id.clone());
        assert_eq!(run_agent_task(&task_id, "exhausted budget", "must not execute", &parent_id,
            7, original.worker_id.clone(), lease, 60_000, config,
            RuntimeExecutionControl::default(), None), 0);
        let after = application_store.runtime_task(&task_id).unwrap().unwrap();
        assert_eq!(after.scheduling().unwrap().unwrap().budget_exhaustion.as_deref(), Some("wall_time_ms"));
        assert_eq!(after.status, "sealed", "budget stop must settle through the original Core Run");
        let receipt = store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap();
        let inspection: nola_domain::RunRecoveryInspection = application.execute(
            nola_application::ApplicationCommand::InspectRunRecovery {
                conversation_id: child_id.clone(), expected_run_id: child_run.begin.run_id.clone(),
                assistant_message_id: "fixture-assistant".into(),
            }).unwrap();
        assert!(matches!(inspection.observation,
            nola_domain::RunRecoveryObservation::Terminal { result, .. }
            if matches!(&result.value, nola_domain::run_result::RunResultValue::Cancelled { cancellation_id, .. }
                if cancellation_id == &format!("task-budget-stop:{task_id}"))));
        assert!(matches!(&receipt.source,
            nola_application::RuntimeTaskTerminalSource::BoundRunTerminal { binding, budget_failure: Some(failure), .. }
            if binding.run_id == child_run.begin.run_id
                && failure.resource == nola_domain::runtime_task_scheduling::RuntimeTaskBudgetResource::WallTimeMs
                && failure.limit == 1));
        let after_effects = effects();
        let confirmations = db.prepare(
            "SELECT payload_json FROM host_event_inbox WHERE run_id=?1"
        ).unwrap().query_map([&child_run.begin.run_id], |row| row.get::<_, String>(0))
            .unwrap().collect::<Result<Vec<_>, _>>().unwrap().into_iter()
            .map(|record| serde_json::from_str::<HostResponseEnvelope>(&record).unwrap())
            .collect::<Vec<_>>();
        for before in &before_effects {
            // Settled outbox rows are a delivery queue, not the historical
            // evidence ledger: a Store reopen prunes acknowledged requests
            // once Core has consumed them. Any retained row must be unchanged;
            // the committed cancellation confirmation identifies the original
            // target even after pruning, and zero attempts prove no dispatch.
            if let Some(after) = after_effects.iter().find(|effect| effect.0 == before.0) {
                assert_eq!((&after.1, &after.3), (&before.1, &before.3));
                assert_eq!(after.2, "acknowledged");
            }
            assert!(confirmations.iter().any(|response| matches!(&response.payload,
                HostResponse::CancellationConfirmed { target_effect_id } if target_effect_id.0 == before.0)),
                "original effect must have a committed cancellation confirmation");
        }
        assert_eq!(delivery_count(), before_deliveries);
        let reopened = NolaStore::open(&database).unwrap();
        assert_eq!(reconcile_task_budget_stop(&reopened.runtime_task(&task_id).unwrap().unwrap()).unwrap(), Some(true));
        assert_eq!(store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap(), receipt,
            "cold reconciliation must replay exactly the original terminal receipt");
        let runs: i64 = db.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id=?1", [&child_id], |row| row.get(0)).unwrap();
        assert_eq!(runs, 1);
        assert!(crate::runtime_task_port::runtime_task_lease_state(&task_id).is_err());
        return;
    }
    if mode == 10 {
        let db = rusqlite::Connection::open(&database).unwrap();
        // Start with a real admitted, bound Run and its pending model effect.
        // This exercises worker recovery, not just the policy parser.
        let deliveries = || db.query_row("SELECT COUNT(*) FROM effect_delivery_attempts", [],
            |row| row.get::<_, i64>(0)).unwrap();
        let runs = || db.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id=?1", [&child_id],
            |row| row.get::<_, i64>(0)).unwrap();
        let effects = || db.prepare(
            "SELECT effect_id,request_id,status,payload_digest FROM effect_outbox WHERE run_id=?1 ORDER BY effect_id"
        ).unwrap().query_map([&child_run.begin.run_id], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?,
        ))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(runs(), 1);
        assert!(!effects().is_empty(), "fixture must have actual pending Host work");
        for metadata in ["{}", "null", "[]", r#"{"_nola_scheduler":{"priority":"urgent"}}"#] {
            db.execute("UPDATE durable_tasks SET metadata_json=?2 WHERE id=?1",
                rusqlite::params![task_id, metadata]).unwrap();
            assert!(!crate::runtime_task_port::reconcile_bound_run_terminal(&task_id, unix_epoch_millis()).unwrap(),
                "fixture must reach budget admission rather than terminal replay");
            let before = store.runtime_task(&task_id).unwrap().unwrap();
            let before_events = store.runtime_task_events(&task_id, 100).unwrap();
            let before_effects = effects();
            let before_deliveries = deliveries();
            let lease = crate::runtime_task_port::register_runtime_task_attempt(original.clone());
            let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
            config.session.session_id = Some(child_id.clone());
            assert_eq!(run_agent_task(&task_id, "damaged budget", "must not execute", &parent_id,
                7, original.worker_id.clone(), lease, 60_000, config,
                RuntimeExecutionControl::default(), None), 2);
            assert_eq!(store.runtime_task(&task_id).unwrap().unwrap(), before);
            assert_eq!(store.runtime_task_events(&task_id, 100).unwrap(), before_events);
            assert_eq!(runs(), 1, "recovery must not create a replacement Run");
            assert_eq!(effects(), before_effects, "recovery must not create or acknowledge model effects");
            assert_eq!(deliveries(), before_deliveries, "recovery must not dispatch existing model work");
            assert!(crate::runtime_task_port::runtime_task_lease_state(&task_id).is_err(),
                "refused worker must release its process-local lease handle");
        }
        return;
    }
    if mode == 9 {
        let mut frozen = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
        frozen.model_settings.model = "frozen-model".into();
        let mut requested = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
        requested.model_settings.model = "different-model".into();
        assert!(crate::core_agent_runtime::resumed_execution_config(
            &frozen, &requested, &child_id, &child_run.begin.run_id).is_err(),
            "cold task child cannot borrow authority merely because no Host was cached");
        assert_eq!(crate::core_agent_runtime::resumed_execution_config(
            &frozen, &requested, &parent_id, "ordinary-run").unwrap().model_settings.model, "frozen-model");
        requested.session.host_task_execution = Some(authority.clone());
        let revision = lease.snapshot().unwrap().revision;
        assert!(crate::core_agent_runtime::resumed_execution_config(
            &frozen, &requested, &child_id, "foreign-run").is_err());
        assert_eq!(lease.snapshot().unwrap().revision, revision,
            "foreign Run adoption must not renew a worker lease");
        let adopted = crate::core_agent_runtime::resumed_execution_config(
            &frozen, &requested, &child_id, &child_run.begin.run_id).unwrap();
        assert_eq!(adopted.model_settings.model, "frozen-model");
        assert!(adopted.session.host_task_execution.as_ref().unwrap().owns(&lease));
        assert!(lease.snapshot().unwrap().revision > revision);
        requested.session.host_task_execution = None;
        assert!(crate::core_agent_runtime::resumed_execution_config(
            &adopted, &requested, &child_id, &child_run.begin.run_id).is_err(),
            "cache presence must not supply omitted worker authority");
        requested.session.host_task_execution = Some(authority.clone());
        authority.revoke();
        assert!(crate::core_agent_runtime::resumed_execution_config(
            &adopted, &requested, &child_id, &child_run.begin.run_id).is_err());
        return;
    }
    if mode == 8 {
        // The public entry must work with durable ownership alone. No local
        // AgentTaskHandle is installed, as with a different worker process.
        assert!(!task_handles().lock().unwrap().contains_key(&task_id));
        assert!(queue_agent_guidance("foreign-parent", &parent_turn, &task_id, "No").is_err());
        let mut messages = Vec::new();
        let mut explicit_rejections=0;
        for index in 0..40 {
            match queue_agent_guidance(&parent_id, &parent_turn, &task_id,
                &format!("Continue with durable guidance {index}")) {
                Ok(id)=>messages.push(id),
                Err(error)=>{assert!(error.contains("task_guidance_queue_full"),"{error}");explicit_rejections+=1;}
            }
        }
        assert_eq!(messages.len(),nola_core::agent::task::MAX_TASK_GUIDANCE_QUEUE);
        assert_eq!(explicit_rejections,40-messages.len());
        drop(store);
        store = nola_store::NolaStore::open(&database).unwrap();
        let records=store.runtime_task_guidance_records(&task_id).unwrap();
        assert_eq!(records.len(),40,"every rejected admission must remain a durable outcome");
        messages.extend(records.iter().filter(|record|record.status=="rejected").map(|record|record.message_id.clone()));
        for message in &messages {
            let record=store.runtime_task_guidance(&task_id,message).unwrap().unwrap();
            assert!(matches!(record.status.as_str(),"accepted"|"rejected"),"global owner decides admission once");
            assert_eq!(record.run_id,child_run.begin.run_id);
        }
        crate::core_agent_runtime::drain_parent_agent_task_inbox(&parent_id).unwrap();
        drop(store);
        let reopened = nola_store::NolaStore::open(&database).unwrap();
        let mut accepted = 0;
        let mut rejected = 0;
        for message in &messages {
            let record = reopened.runtime_task_guidance(&task_id, message).unwrap().unwrap();
            match record.status.as_str() {
                "accepted" => accepted += 1,
                "rejected" => {
                    rejected += 1;
                    assert_eq!(record.reason.as_deref(), Some("task_guidance_queue_full"));
                }
                status => panic!("guidance was not durably settled at admission: {status}"),
            }
        }
        assert_eq!(accepted, nola_core::agent::task::MAX_TASK_GUIDANCE_QUEUE);
        assert_eq!(accepted + rejected, 40);
        assert!(rejected > 0, "fixture must exercise bounded admission, not silent loss");
        let mut child = crate::session::SessionStore::open(
            Some(&child_id), false, fixture._root.path().to_str().unwrap()).unwrap();
        assert!(child.apply_task_guidance(&task_id, &messages[0], &child_run.begin.run_id).is_err(),
            "an active Run pointer alone must not authorize guidance delivery");
        assert_eq!(reopened.runtime_task_guidance(&task_id, &messages[0]).unwrap().unwrap().status, "accepted");
        child.set_task_execution_authority(Some(authority));
        let delivered = child.apply_task_guidance(&task_id, &messages[0], &child_run.begin.run_id).unwrap();
        assert_eq!(delivered.guidance.status, "delivered");
        assert!(delivered.accepted.is_some());
        // Simulate losing the Host acknowledgement: reopen the real SessionStore
        // and reapply without worker authority. Only observation is permitted.
        drop(child);
        let mut child = crate::session::SessionStore::open(
            Some(&child_id), false, fixture._root.path().to_str().unwrap()).unwrap();
        let replay = child.apply_task_guidance(&task_id, &messages[0], &child_run.begin.run_id).unwrap();
        assert_eq!(replay.guidance.delivery_transition_id, delivered.guidance.delivery_transition_id);
        assert!(replay.accepted.is_some());
        assert_eq!(delivered.accepted.as_ref().unwrap().control.queued_input_count, 1);
        assert_eq!(replay.accepted.as_ref().unwrap().control.queued_input_count, 1,
            "acknowledgement recovery must not inject the guidance twice");
        let connection = rusqlite::Connection::open(&database).unwrap();
        let run_count: i64 = connection.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id=?1",
            [&child_id], |row| row.get(0)).unwrap();
        assert_eq!(run_count, 1, "guidance or its replay must never start another Run");
        return;
    }
    if mode == 6 {
        let db = rusqlite::Connection::open(&database).unwrap();
        // Model missing historical evidence, not a supported terminal writer.
        db.execute("UPDATE durable_tasks SET status='completed',completion_watermark=7,ownership_token=NULL WHERE id=?1",
            [&task_id]).unwrap();
        let task = NolaStore::open_default().unwrap().runtime_task(&task_id).unwrap().unwrap();
        let count = || db.query_row("SELECT COUNT(*) FROM agent_commands", [], |row| row.get::<_, i64>(0)).unwrap();
        let before = count();
        assert!(!replay_terminal_agent_reconciliation(&task));
        assert_eq!(count(), before, "missing terminal receipt borrowed the parent's current token");
        return;
    }
    if mode == 5 {
        // Produce the terminal fact through the real admission/reducer/journal
        // transaction, then lose the worker acknowledgement at this boundary.
        let db = rusqlite::Connection::open(&database).unwrap();
        let claimed_at = unix_epoch_millis();
        let claimed = application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: child_run.begin.run_id.clone(), worker_generation: original.worker_generation.clone(),
            lease_owner: original.worker_id.clone(), claimed_at_ms: claimed_at,
            lease_expires_at_ms: claimed_at + 60_000, limit: 1,
        }).unwrap();
        let request = claimed.requests.into_iter().next().expect("original child model request");
        assert!(matches!(request.payload, HostRequest::ModelStream { .. }));
        let mut sequence = request.resume_after_sequence.unwrap_or(0);
        for (index, payload) in [
            HostResponse::ExecutionStarted,
            HostResponse::ModelAttemptStarted { provider: "fixture".into(), model: "fixture-model".into(), reason: "initial".into() },
            HostResponse::ModelCompleted {
                items: vec![serde_json::to_value(nola_core::agent::AgentItem::assistant_message(
                    nola_core::agent::ItemId::from_string("actual-child-result".into()), "Completed child result",
                )).unwrap()], tool_calls: vec![], usage: json!({}),
            },
        ].into_iter().enumerate() {
            sequence += 1;
            application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                conversation_id: child_id.clone(), run_id: child_run.begin.run_id.clone(),
                invocation_config_digest: child_run.invocation_config_digest.clone(),
                response: HostResponseEnvelope {
                    request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
                    host_id: request.host_id.clone(), lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                    attempt_id: request.attempt_id.clone(), delegated_invocation_id: request.delegated_invocation_id.clone(),
                    delivery: request.delivery.clone(), sequence, payload,
                }, received_at_ms: claimed_at + index as i64 + 1,
            }).unwrap();
        }
        assert!(application.active_run(&child_id).unwrap().is_none(), "fixture must reach actual Core completion");
        drop(application);
        drop(store);
        let mut first_terminal_command = None;
        for _ in 0..2 {
            let mut process = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "task_runtime::permission_wait_tests::bound_terminal_worker_child", "--nocapture"])
                .env("NOLA_TEST_BOUND_TERMINAL_ATTEMPT", serde_json::to_string(&original).unwrap())
                .env("NOLA_TEST_BOUND_TERMINAL_CHILD", &child_id)
                .env("NOLA_TEST_BOUND_TERMINAL_OWNER", &parent_id)
                .stderr(std::process::Stdio::piped())
                .spawn().unwrap();
            // Drain concurrently: a failed child must not deadlock on its
            // stderr pipe. Retain only a bounded fixture diagnostic, not an
            // entire checkpoint or model transcript.
            let mut stderr = process.stderr.take().unwrap();
            let diagnostics = std::thread::spawn(move || {
                use std::io::Read;
                let mut kept = Vec::new();
                let mut buffer = [0_u8; 1024];
                loop {
                    let count = stderr.read(&mut buffer).unwrap();
                    if count == 0 { break; }
                    let retain = count.min(8192_usize.saturating_sub(kept.len()));
                    kept.extend_from_slice(&buffer[..retain]);
                }
                String::from_utf8_lossy(&kept).into_owned()
            });
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(status) = process.try_wait().unwrap() {
                    let diagnostic = diagnostics.join().unwrap();
                    assert!(status.success(), "terminal recovery process failed: {status}\n{diagnostic}");
                    break;
                }
                if Instant::now() >= deadline {
                    process.kill().unwrap();
                    let _ = process.wait();
                    let diagnostic = diagnostics.join().unwrap();
                    panic!("terminal recovery process timed out\n{diagnostic}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let store = nola_store::NolaStore::open(&database).unwrap();
            let task = store.runtime_task(&task_id).unwrap().unwrap();
            assert_eq!(task.status, "sealed");
            assert_eq!(task.completion_watermark, Some(7));
            assert!(task.ownership_token.is_none());
            assert_eq!(store.runtime_task_events(&task_id, 100).unwrap().iter()
                .filter(|event| event.kind == "bound_run_terminal").count(), 1);
            let run_count: i64 = db.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id=?1",
                [&child_id], |row| row.get(0)).unwrap();
            assert_eq!(run_count, 1, "terminal recovery started a replacement Run");
            let queued: Vec<(String, String)> = db.prepare(
                "SELECT command_id,payload_json FROM agent_commands WHERE session_id=?1 ORDER BY sequence",
            ).unwrap().query_map([&parent_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap().collect::<Result<_, _>>().unwrap();
            assert_eq!(queued.len(), 1, "terminal receipt must enqueue exactly one parent command");
            if let Some(first) = &first_terminal_command {
                assert_eq!(&queued[0], first, "restart changed the terminal operation identity or payload");
            } else {
                first_terminal_command = Some(queued[0].clone());
            }
        }
        return;
    }
    task_handles().lock().unwrap().insert(task_id.clone(), AgentTaskHandle {
        control_id: task_id.clone(), lease: Some(lease.clone()), execution: Some(authority.clone()),
        permission_resume: None,
        cancellation: fixture.cancellation.clone(), child_session_id: child_id.clone(),
    });
    if mode == 20 {
        let HostRequest::RequestPermission { capability_id, input, .. } =
            &core_permission.as_ref().unwrap().payload else { unreachable!() };
        let request = tools::ToolRequest {
            id: "prior-host-grant".into(), name: capability_id.clone(),
            args: input.as_object().unwrap().clone(),
        };
        let rule = tools::session_permission_rule(&request);
        assert!(tools::session_permission_rule_matches(&rule, &request));
        crate::permission_grants::add_permission_grant(
            &format!("host:task:{task_id}"), "tool", "allow_rule", &rule, "allow",
        ).unwrap();
        // Even an exact Host-rule match cannot skip the original suspension
        // and its explicit Store decision tested below. Reuse is Core-owned.
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let settlement_request = core_permission.clone();
    let mut _cold_startup_heartbeat = None;
    if cold {
        let at = unix_epoch_millis();
        let suspend_input = nola_application::SuspendRuntimeTaskForPermission {
            task_id: task_id.clone(), attempt_id: original.id.clone(), expected_attempt_revision: original.revision,
            worker_generation: original.worker_generation.clone(), lease_token: original.lease_token.clone(),
            current_expiry_ms: original.lease_expires_at_ms,
            permission_request: nola_store_port::permission::StoredPermissionRequest {
                closure: None,
                request_id: "fixture-cold-permission".into(), task_id: task_id.clone(), child_session_id: child_id.clone(),
                capability: "file.write".into(), input_json: json!({"path":"fixture-output.txt","content":"test"}).to_string(),
                scope_rule: "fixture-rule".into(), created_at_ms: at, core_permission_request_id: Some(original_core_permission_id.clone()),
                permission_instructions_json: "[]".into(), status: "pending".into(), decision: None,
                custom_response: None, resolved_at_ms: None,
            },
            continuation_id: "fixture-cold-continuation".into(),
            continuation_json: json!({"child_session_id":child_id,"run_id":child_run.begin.run_id,
                "core_permission_request_id":original_core_permission_id, "capability":"file.write",
                "input":{"path":"fixture-output.txt","content":"test"},"input_watermark":7}).to_string(),
            progress_summary: "Waiting".into(), last_activity: None, now_ms: at,
        };
        use nola_store_port::permission::PermissionRepository;
        for ensure in [false, true] {
            let mut invalid = suspend_input.permission_request.clone();
            invalid.input_json = json!({"path":"forged-resource"}).to_string();
            let result = if ensure {
                store.ensure_core_permission_request(&invalid).map(|_| ())
            } else {
                store.enqueue_permission_request(&invalid)
            };
            assert!(result.is_err(), "standalone admission must validate the actual Core effect");
            assert!(store.permission_request(&invalid.request_id).unwrap().is_none());
        }
        let inspect = rusqlite::Connection::open(&database).unwrap();
        inspect.execute_batch("CREATE TRIGGER fail_source BEFORE INSERT ON permission_request_sources
            BEGIN SELECT RAISE(ABORT,'source fault'); END").unwrap();
        assert!(store.ensure_core_permission_request(&suspend_input.permission_request).is_err());
        assert!(store.permission_request(&suspend_input.permission_request.request_id).unwrap().is_none());
        inspect.execute_batch("DROP TRIGGER fail_source").unwrap();
        store.ensure_core_permission_request(&suspend_input.permission_request).unwrap();
        assert!(store.latest_runtime_task_permission_suspension(&task_id).unwrap().is_none());
        let retained: String = inspect.query_row("SELECT proof_json FROM permission_request_sources WHERE request_id=?1",
            [&suspend_input.permission_request.request_id], |row| row.get(0)).unwrap();
        if matches!(mode, 16..=19) {
            let approved = mode >= 18;
            let candidate = &suspend_input.permission_request;
            crate::permission_request_port::authorize_permission(&candidate.request_id,
                if approved { nola_application::PermissionAuthorizationDecision::AllowConversation }
                else { nola_application::PermissionAuthorizationDecision::Deny }, unix_epoch_millis()).unwrap();
            let before = store.permission_request(&candidate.request_id).unwrap().unwrap();
            let grant_id = format!("permission-request:{}:task-scope", candidate.request_id);
            let live_grants = || inspect.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM permission_grants WHERE id=?1 AND revoked_at_ms IS NULL",
                [&grant_id], |row| row.get(0)).unwrap();
            assert_eq!(live_grants(), i64::from(approved));
            let deliver = |application: &mut crate::application_port::CliApplicationPort,
                           request: &HostRequestEnvelope, payloads: Vec<HostResponse>| {
                let base = request.resume_after_sequence.unwrap_or(0);
                for (index, payload) in payloads.into_iter().enumerate() {
                    application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                        conversation_id: child_id.clone(), run_id: child_run.begin.run_id.clone(),
                        invocation_config_digest: child_run.invocation_config_digest.clone(),
                        received_at_ms: unix_epoch_millis(),
                        response: HostResponseEnvelope {
                            request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
                            host_id: request.host_id.clone(), lease_id: request.lease_id.clone(),
                            session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                            catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                            attempt_id: request.attempt_id.clone(), delegated_invocation_id: request.delegated_invocation_id.clone(),
                            delivery: request.delivery.clone(), sequence: base + index as u64 + 1, payload,
                        },
                    }).unwrap();
                }
            };
            let claim = |application: &mut crate::application_port::CliApplicationPort|
                application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
                run_id: child_run.begin.run_id.clone(), worker_generation: original.worker_generation.clone(),
                lease_owner: original.worker_id.clone(), claimed_at_ms: unix_epoch_millis(),
                lease_expires_at_ms: unix_epoch_millis() + 60_000, limit: 16,
            }).unwrap().requests;
            let permission = claim(&mut application).into_iter().find(|request|
                matches!(request.payload, HostRequest::RequestPermission { .. })).unwrap();
            deliver(&mut application, &permission, vec![HostResponse::ExecutionStarted, HostResponse::PermissionResolved {
                request_id: original_core_permission_id.clone(), resolution: if approved {
                    HostPermissionResolution::allow_conversation(None)
                } else { HostPermissionResolution::deny() },
            }]);
            let settled = store.permission_request(&candidate.request_id).unwrap().unwrap();
            assert_eq!(settled.status, "withdrawn");
            assert_eq!(settled.decision, before.decision);
            assert_eq!(settled.resolved_at_ms, before.resolved_at_ms);
            assert!(matches!(&settled.closure.as_ref().unwrap().source,
                nola_domain::permission_closure::PermissionClosureSource::CoreSettled { permission, .. }
                if permission.core_permission_request_id == original_core_permission_id));
            if approved {
                let tool = claim(&mut application).into_iter().find(|request|
                    matches!(request.payload, HostRequest::ExecuteTool { .. })).unwrap();
                let HostRequest::ExecuteTool { call_id, .. } = &tool.payload else { unreachable!() };
                // Simulated Host completion; this fixture exercises authority
                // and closure, not filesystem side effects or receipt evidence.
                deliver(&mut application, &tool, vec![HostResponse::ExecutionStarted, HostResponse::ToolCompleted {
                    call_id: call_id.clone(), output: json!({"fixture":true}),
                    outcome: if mode == 19 { HostToolOutcome::Failed } else { HostToolOutcome::Succeeded },
                    activation: None, full_output_ref: None, artifacts: vec![],
                }]);
            }
            let model = claim(&mut application).into_iter().find(|request|
                matches!(request.payload, HostRequest::ModelStream { .. })).unwrap();
            let end = if matches!(mode, 17 | 19) {
                HostResponse::Failed { error: HostError { code: "fixture_terminal_failure".into(),
                    message: "Non-retryable fixture failure".into(), retryable: false, http_status: None } }
            } else {
                HostResponse::ModelCompleted { items: vec![serde_json::to_value(
                    nola_core::agent::AgentItem::assistant_message(
                        nola_core::agent::ItemId::from_string("permission-fixture-answer".into()),
                        "Fixture run ended; see the tool result.")).unwrap()],
                    tool_calls: vec![], usage: json!({}) }
            };
            deliver(&mut application, &model, vec![HostResponse::ExecutionStarted,
                HostResponse::ModelAttemptStarted { provider: "fixture".into(), model: "fixture-model".into(), reason: "initial".into() }, end]);
            assert!(application.active_run(&child_id).unwrap().is_none());
            assert!(store.latest_runtime_task_permission_suspension(&task_id).unwrap().is_none());
            inspect.execute_batch("CREATE TRIGGER fail_non_cancel_close BEFORE INSERT ON runtime_task_terminal_receipts
                BEGIN SELECT RAISE(ABORT,'terminal fault'); END").unwrap();
            assert!(crate::runtime_task_port::reconcile_bound_run_terminal(&task_id, unix_epoch_millis()).is_err());
            assert!(store.runtime_task_terminal_receipt(&task_id).unwrap().is_none());
            assert_eq!(store.permission_request(&candidate.request_id).unwrap().unwrap(), settled);
            assert_eq!(live_grants(), i64::from(approved), "failed terminal transaction must roll back grant revocation");
            inspect.execute_batch("DROP TRIGGER fail_non_cancel_close").unwrap();
            assert!(crate::runtime_task_port::reconcile_bound_run_terminal(&task_id, unix_epoch_millis()).unwrap());
            drop(store);
            store = nola_store::NolaStore::open(&database).unwrap();
            let receipt = store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap();
            let seal = &receipt.journal_result.as_ref().unwrap().seal;
            assert_eq!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Failed { .. }), matches!(mode, 17 | 19));
            assert!(!matches!(seal.result.value, nola_domain::run_result::RunResultValue::Cancelled { .. }));
            let closed = store.permission_request(&candidate.request_id).unwrap().unwrap();
            assert_eq!(closed.status, "withdrawn");
            assert_eq!(closed.decision, before.decision);
            assert_eq!(closed.resolved_at_ms, before.resolved_at_ms);
            assert_eq!(live_grants(), 0);
            if approved {
                assert_eq!(inspect.query_row::<i64, _, _>(
                    "SELECT COUNT(*) FROM permission_grants WHERE id=?1 AND revoked_at_ms IS NOT NULL",
                    [&grant_id], |row| row.get(0)).unwrap(), 1, "retain revoked grant history");
            }
            assert_eq!(closed, settled, "terminal reconciliation preserves the original Core settlement");
            assert_eq!(store.ensure_core_permission_request(candidate).unwrap(), closed);
            assert!(crate::runtime_task_port::reconcile_bound_run_terminal(&task_id, unix_epoch_millis()).unwrap());
            assert_eq!(store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap(), receipt);
            assert_eq!(inspect.query_row::<String, _, _>("SELECT proof_json FROM permission_request_sources WHERE request_id=?1",
                [&candidate.request_id], |row| row.get(0)).unwrap(), retained);
            assert_eq!(inspect.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs WHERE conversation_id=?1",
                [&child_id], |row| row.get(0)).unwrap(), 1);
            task_handles().lock().unwrap().remove(&task_id);
            return;
        }
        if matches!(mode, 14 | 15) {
            let candidate = &suspend_input.permission_request;
            if mode == 15 {
                assert!(matches!(crate::permission_request_port::authorize_permission(&candidate.request_id,
                    nola_application::PermissionAuthorizationDecision::AllowConversation, unix_epoch_millis()).unwrap(),
                    nola_application::PermissionResolutionOutcome::Resolved { .. }));
            }
            let before = store.permission_request(&candidate.request_id).unwrap().unwrap();
            assert!(crate::runtime_task_port::request_cancel(&task_id, unix_epoch_millis()).unwrap());
            assert_eq!(store.permission_request(&candidate.request_id).unwrap().unwrap(), before);
            assert!(store.runtime_task_terminal_receipt(&task_id).unwrap().is_none());
            assert!(store.effective_permission_grants(None, Some(&format!("task:{task_id}"))).unwrap().is_empty());
            let task = store.runtime_task(&task_id).unwrap().unwrap();
            assert_eq!(task.status, "running");
            authority.revoke();
            task_handles().lock().unwrap().remove(&task_id);
            drop(store);
            let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
            config.tool_settings.project_root = task.project_root.clone().into();
            config.session.session_id = task.owner_session_id;
            inspect.execute_batch("CREATE TRIGGER fail_standalone_close BEFORE UPDATE ON permission_requests
                WHEN NEW.closure_json IS NOT NULL BEGIN SELECT RAISE(ABORT,'standalone closure fault'); END").unwrap();
            recover_orphaned_agent_tasks(&config);
            store = nola_store::NolaStore::open(&database).unwrap();
            assert!(store.runtime_task_terminal_receipt(&task_id).unwrap().is_none());
            assert_eq!(store.runtime_task(&task_id).unwrap().unwrap().status, "running");
            assert_eq!(store.permission_request(&candidate.request_id).unwrap().unwrap(), before);
            inspect.execute_batch("DROP TRIGGER fail_standalone_close").unwrap();
            recover_orphaned_agent_tasks(&config);
            drop(store);
            store = nola_store::NolaStore::open(&database).unwrap();
            let receipt = store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap();
            let seal = &receipt.journal_result.as_ref().unwrap().seal;
            assert_eq!(receipt.status, "sealed");
            assert_eq!(seal.identity.run_id, child_run.begin.run_id);
            assert!(matches!(&seal.result.value, nola_domain::run_result::RunResultValue::Cancelled { .. }));
            let closed = store.permission_request(&candidate.request_id).unwrap().unwrap();
            let proof: nola_domain::permission_closure::CorePermissionSourceProof = serde_json::from_str(&retained).unwrap();
            assert_eq!(closed.status, "withdrawn");
            assert_eq!(closed.decision, before.decision);
            assert_eq!(closed.resolved_at_ms, before.resolved_at_ms);
            assert!(matches!(&closed.closure.as_ref().unwrap().source,
                nola_domain::permission_closure::PermissionClosureSource::RunSealed { permission, seal_digest, .. }
                    if permission == &proof.permission && seal_digest == &seal.seal_digest));
            assert!(store.latest_runtime_task_permission_suspension(&task_id).unwrap().is_none());
            assert!(store.pending_permission_for_task(&task_id).unwrap().is_none());
            assert_eq!(store.ensure_core_permission_request(candidate).unwrap(), closed);
            assert!(matches!(crate::permission_request_port::authorize_permission(&candidate.request_id,
                nola_application::PermissionAuthorizationDecision::AllowOnce, unix_epoch_millis()).unwrap(),
                nola_application::PermissionResolutionOutcome::Closed { .. }));
            recover_orphaned_agent_tasks(&config);
            assert_eq!(store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap(), receipt);
            assert_eq!(inspect.query_row::<String, _, _>("SELECT proof_json FROM permission_request_sources WHERE request_id=?1",
                [&candidate.request_id], |row| row.get(0)).unwrap(), retained);
            assert_eq!(inspect.query_row::<i64, _, _>("SELECT COUNT(*) FROM effect_outbox WHERE effect_id=?1",
                [&proof.permission.effect_id], |row| row.get(0)).unwrap(), 0);
            assert_eq!(inspect.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs WHERE conversation_id=?1",
                [&child_id], |row| row.get(0)).unwrap(), 1);
            assert_eq!(receipt.attempts.len(), 1);
            assert_eq!(receipt.attempts[0].id, original.id);
            return;
        }
        let suspension = store.suspend_runtime_task_for_permission(&suspend_input).unwrap().unwrap();
        let source: nola_domain::permission_closure::CorePermissionSourceProof = serde_json::from_value(
            serde_json::from_str::<Value>(&suspension.continuation_json).unwrap()["core_permission_source"].clone()
        ).unwrap();
        assert_eq!(serde_json::from_str::<nola_domain::permission_closure::CorePermissionSourceProof>(&retained).unwrap(), source);
        assert!(inspect.execute("UPDATE permission_request_sources SET proof_json=proof_json WHERE request_id=?1",
            [&suspend_input.permission_request.request_id]).is_err());
        assert_eq!(source.permission.run_id, child_run.begin.run_id);
        assert_eq!(source.permission.core_permission_request_id, original_core_permission_id);
        assert_eq!(source.permission.effect_id, core_permission.as_ref().unwrap().effect_id.0);
        authority.revoke();
        task_handles().lock().unwrap().remove(&task_id);
        drop(store);
        store = nola_store::NolaStore::open(&database).unwrap();
        assert_eq!(store.latest_runtime_task_permission_suspension(&task_id).unwrap().unwrap().continuation_json,
            suspension.continuation_json, "source proof must survive a cold reopen unchanged");
        if mode == 12 {
            let db = rusqlite::Connection::open(&database).unwrap();
            let application_store = NolaStore::open(&database).unwrap();
            let mut waiting = application_store.runtime_task(&task_id).unwrap().unwrap();
            let mut scheduling = waiting.scheduling().unwrap().unwrap();
            scheduling.resource_budget.max_wall_time_ms = Some(1);
            waiting.set_scheduling(scheduling).unwrap();
            db.execute("UPDATE durable_tasks SET metadata_json=?2,started_at_ms=?3 WHERE id=?1",
                rusqlite::params![task_id, waiting.metadata_json, now - 1000]).unwrap();
            let waiting = application_store.runtime_task(&task_id).unwrap().unwrap();
            assert_eq!(waiting.status, "waiting_permission");
            assert!(waiting.ownership_token.is_none());
            assert!(waiting.scheduling().unwrap().unwrap().budget_exhaustion.is_none());
            let attempts_before: i64 = db.query_row("SELECT COUNT(*) FROM runtime_task_attempts WHERE task_id=?1",
                [&task_id], |row| row.get(0)).unwrap();
            let original_effects = db.prepare("SELECT effect_id FROM effect_outbox WHERE run_id=?1")
                .unwrap().query_map([&child_run.begin.run_id], |row| row.get::<_, String>(0))
                .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
            assert!(!original_effects.is_empty());
            assert_eq!(reconcile_task_budget_stop(&waiting).unwrap(), Some(true));
            let settled = application_store.runtime_task(&task_id).unwrap().unwrap();
            assert_eq!(settled.status, "sealed");
            assert_eq!(settled.scheduling().unwrap().unwrap().budget_exhaustion.as_deref(), Some("wall_time_ms"));
            assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runtime_task_attempts WHERE task_id=?1",
                [&task_id], |row| row.get(0)).unwrap(), attempts_before);
            let receipt = store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap();
            assert!(matches!(&receipt.source,
                nola_application::RuntimeTaskTerminalSource::BoundRunTerminal { binding, budget_failure: Some(failure), .. }
                if binding.run_id == child_run.begin.run_id
                    && failure.resource == nola_domain::runtime_task_scheduling::RuntimeTaskBudgetResource::WallTimeMs));
            assert_eq!(reconcile_task_budget_stop(&settled).unwrap(), Some(true));
            assert_eq!(store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap(), receipt);
            assert_eq!(store.runtime_task_events(&task_id, 100).unwrap().iter()
                .filter(|event| event.kind == "budget_exhausted").count(), 1);
            for effect in original_effects {
                assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM effect_delivery_attempts WHERE effect_id=?1",
                    [&effect], |row| row.get(0)).unwrap(), 0, "permission budget stop must not dispatch model work");
            }
            assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs WHERE conversation_id=?1",
                [&child_id], |row| row.get(0)).unwrap(), 1);
            return;
        }
        if matches!(mode, 7 | 13) {
            if mode == 13 {
                assert!(matches!(crate::permission_request_port::authorize_permission(&suspension.permission_request_id,
                    nola_application::PermissionAuthorizationDecision::AllowConversation, unix_epoch_millis()).unwrap(),
                    nola_application::PermissionResolutionOutcome::Resolved { .. }));
            }
            let before_stop = crate::permission_request_port::permission_request(&suspension.permission_request_id).unwrap().unwrap();
            assert!(crate::runtime_task_port::request_cancel(&task_id, unix_epoch_millis()).unwrap());
            assert!(store.runtime_task_terminal_receipt(&task_id).unwrap().is_none());
            let request = crate::permission_request_port::permission_request(&suspension.permission_request_id)
                .unwrap().unwrap();
            assert_eq!(request, before_stop, "stop intent must not fabricate or replace a user decision");
            let waiting = store.runtime_task(&task_id).unwrap().unwrap();
            drop(store);
            let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
            config.tool_settings.project_root = waiting.project_root.clone().into();
            config.session.session_id = waiting.owner_session_id;
            if mode == 7 {
                let db = rusqlite::Connection::open(&database).unwrap();
                db.execute_batch("CREATE TRIGGER fail_permission_closure BEFORE UPDATE ON permission_requests
                    WHEN NEW.status='withdrawn' BEGIN SELECT RAISE(ABORT,'closure-write-fault'); END;").unwrap();
                recover_orphaned_agent_tasks(&config);
                let failed = nola_store::NolaStore::open(&database).unwrap();
                assert!(failed.runtime_task_terminal_receipt(&task_id).unwrap().is_none());
                assert_eq!(failed.runtime_task(&task_id).unwrap().unwrap().status, "waiting_permission");
                assert_eq!(crate::permission_request_port::permission_request(&suspension.permission_request_id).unwrap().unwrap(), before_stop);
                assert_eq!(failed.latest_runtime_task_permission_suspension(&task_id).unwrap().unwrap().state, "pending");
                db.execute_batch("DROP TRIGGER fail_permission_closure;").unwrap();
            }
            recover_orphaned_agent_tasks(&config);
            store = nola_store::NolaStore::open(&database).unwrap();
            let receipt = store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap();
            assert_eq!(receipt.status, "sealed");
            assert!(matches!(&receipt.source, nola_application::RuntimeTaskTerminalSource::BoundRunTerminal { binding, .. }
                if binding.run_id == child_run.begin.run_id));
            assert!(matches!(&receipt.journal_result.as_ref().unwrap().seal.result.value,
                nola_domain::run_result::RunResultValue::Cancelled { cancellation_id, .. }
                if cancellation_id == &format!("task-user-input-cancel:{task_id}")));
            assert_eq!(receipt.attempts.len(), 1);
            assert_eq!(receipt.attempts[0].id, original.id);
            assert_eq!(receipt.attempts[0].lease_token, original.lease_token);
            assert_eq!(receipt.input_watermark, Some(7));
            assert_eq!(receipt.completion_watermark, Some(7));
            let closed = crate::permission_request_port::permission_request(&suspension.permission_request_id).unwrap().unwrap();
            assert_eq!(closed.status, nola_application::PermissionRequestStatus::Withdrawn);
            assert_eq!(closed.resolution, before_stop.resolution);
            assert_eq!(closed.resolved_at_ms, before_stop.resolved_at_ms);
            let closure = closed.closure.as_ref().unwrap();
            assert!(matches!(&closure.source, nola_domain::permission_closure::PermissionClosureSource::RunSealed {
                permission, seal_digest, .. } if permission == &source.permission
                    && seal_digest == &receipt.journal_result.as_ref().unwrap().seal.seal_digest));
            assert!(store.pending_permission_for_task(&task_id).unwrap().is_none());
            assert!(background_permission_decision(closed.clone()).is_none());
            assert!(matches!(crate::permission_request_port::authorize_permission(&closed.request_id,
                nola_application::PermissionAuthorizationDecision::AllowOnce, unix_epoch_millis()).unwrap(),
                nola_application::PermissionResolutionOutcome::Closed { request } if request == closed));
            recover_orphaned_agent_tasks(&config);
            assert_eq!(crate::permission_request_port::permission_request(&closed.request_id).unwrap().unwrap(), closed);
            assert_eq!(store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap(), receipt);
            let reopened = nola_store::NolaStore::open(&database).unwrap();
            let retained = reopened.latest_runtime_task_permission_suspension(&task_id).unwrap().unwrap();
            assert_eq!(retained.continuation_json, suspension.continuation_json,
                "terminal reconciliation must retain the captured permission source");
            let db = rusqlite::Connection::open(&database).unwrap();
            assert!(db.execute("UPDATE permission_requests SET decision='deny' WHERE request_id=?1",
                [&closed.request_id]).is_err(), "closed decision history is immutable");
            assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM effect_outbox WHERE effect_id=?1",
                [&source.permission.effect_id], |row| row.get(0)).unwrap(), 0,
                "prove source retention after the consumed delivery queue is pruned");
            assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs WHERE conversation_id=?1",
                [&child_id], |row| row.get(0)).unwrap(), 1);
            assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runtime_task_attempts WHERE task_id=?1",
                [&task_id], |row| row.get(0)).unwrap(), 1);
            // Provenance-owned model requests retain their settled delivery
            // records. They must not become executable when permission closes.
            assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM effect_delivery_attempts a
                JOIN effect_outbox e ON e.effect_id=a.effect_id WHERE e.run_id=?1
                AND (e.status!='acknowledged' OR a.state!='settled')",
                [&child_run.begin.run_id], |row| row.get(0)).unwrap(), 0);
            return;
        }
        assert!(matches!(crate::permission_request_port::authorize_permission(&suspension.permission_request_id,
            nola_application::PermissionAuthorizationDecision::AllowOnce, unix_epoch_millis()).unwrap(),
            nola_application::PermissionResolutionOutcome::Resolved { .. }));
        let task = NolaStore::open_default().unwrap().runtime_task(&task_id).unwrap().unwrap();
        for (pid, expected_error) in [(std::process::id(), "live process"), (0, "unknown process ownership")] {
            let owner = format!("fixture-existing-owner-{pid}");
            let acquired: bool = application.execute(nola_application::ApplicationCommand::AcquireSessionRunLease {
                input: nola_application::SessionRunLease {
                    session_id: child_id.clone(), owner: owner.clone(), pid,
                    process_start: "fixture-process-start".into(), acquired_at_ms: unix_epoch_millis(),
                },
            }).unwrap();
            assert!(acquired);
            assert!(admit_cold_permission_resume(&task).unwrap_err().contains(expected_error));
            assert_eq!(store.pending_runtime_task_permission_suspension(&task_id).unwrap().as_ref(), Some(&suspension),
                "live or unknown owner must not consume the pending permission continuation");
            assert!(store.runtime_task_permission_resume(&task_id, &suspension.id,
                &suspension.permission_request_id, &suspension.continuation_id).unwrap().is_none());
            let released: bool = application.execute(nola_application::ApplicationCommand::ReleaseSessionRunLease {
                session_id: child_id.clone(), owner,
            }).unwrap();
            assert!(released);
        }
        let mut resume = admit_cold_permission_resume(&task).unwrap().unwrap();
        if mode >= 2 {
            let first_grant = resume.grant.clone();
            if mode >= 3 {
                // Reopening the source control adapter never creates a second
                // worker grant or requires copying its lease into parent Core.
                application=crate::application_port::CliApplicationPort::persistent_default().unwrap();
                let parent=crate::session::load_session_task_projection(&parent_id).unwrap().unwrap();
                assert!(parent.tasks[&task_id].durability.ownership_token.is_none());
            }
            let running = NolaStore::open_default().unwrap().runtime_task(&task_id).unwrap().unwrap();
            assert!(admit_cold_permission_resume(&running).unwrap().is_none(), "unexpired grant must not be taken over");
            wait_for_fact("prior grant expiry", || (unix_epoch_millis() >= first_grant.lease_expires_at_ms).then_some(()));
            drop(store);
            store = nola_store::NolaStore::open(&database).unwrap();
            resume = admit_cold_permission_resume(&running).unwrap().unwrap();
            assert_ne!(resume.grant.id, first_grant.id);
            assert_ne!(resume.grant.lease_token, first_grant.lease_token);
            assert_eq!(resume.grant.attempt_number, first_grant.attempt_number + 1);
            assert_eq!(resume.suspension.id, suspension.id);
            assert_eq!(store.runtime_task(&task_id).unwrap().unwrap().retry_count, 1);
            assert_eq!(store.runtime_task_attempt(&first_grant.id).unwrap().unwrap().state, "superseded");
        }
        let fresh_lease = crate::runtime_task_port::register_runtime_task_attempt(resume.grant.clone());
        // The short first grant above deliberately exercises expiry/takeover.
        // Once the successor is held, mirror run_agent_task's startup: renew
        // that exact grant while validation/setup runs. Without this heartbeat
        // the fixture races its own 1s lease during unrelated negative checks.
        let startup_execution = crate::runtime_task_port::RuntimeTaskExecutionAuthority::new(fresh_lease.clone());
        _cold_startup_heartbeat = Some(AgentTaskLeaseHeartbeat::start(
            &task_id, &parent_id, &startup_execution, 60_000, &fixture.cancellation,
        ).unwrap());
        let parent_inbox = || {
            let db = rusqlite::Connection::open(&database).unwrap();
            let mut statement = db.prepare("SELECT command_id,payload_json FROM agent_commands WHERE session_id=?1 ORDER BY sequence").unwrap();
            statement.query_map([&parent_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        };
        let inbox_before_rejected_handoffs = parent_inbox();
        let mut forged = resume.clone();
        forged.original.lease_token = "forged-original-token".into();
        assert!(validate_cold_permission_grant(&forged).unwrap_err().contains("identity changed"));
        forged = resume.clone();
        forged.original.worker_generation = "forged-original-generation".into();
        assert!(validate_cold_permission_grant(&forged).unwrap_err().contains("identity changed"));
        assert!(validate_cold_permission_execution(&parent_id, &task_id, 8, &resume, &fresh_lease,
            &fixture.cancellation).unwrap_err().contains("live exact admitted authority"));
        assert!(validate_cold_permission_execution("wrong-parent", &task_id, 7, &resume, &fresh_lease,
            &fixture.cancellation).unwrap_err().contains("live exact admitted authority"));
        let mut forged_grant = resume.grant.clone();
        forged_grant.lease_token = "forged-successor-token".into();
        let forged_handle = crate::runtime_task_port::register_runtime_task_attempt(forged_grant);
        assert!(validate_cold_permission_execution(&parent_id, &task_id, 7, &resume, &forged_handle,
            &fixture.cancellation).unwrap_err().contains("exact continuation grant"));
        let stopped = RuntimeExecutionControl::default();
        stopped.request();
        assert!(validate_cold_permission_execution(&parent_id, &task_id, 7, &resume, &fresh_lease,
            &stopped).unwrap_err().contains("exact continuation grant"));
        assert_eq!(parent_inbox(), inbox_before_rejected_handoffs,
            "rejected handoff changed the parent inbox");
        if mode == 4 {
            // A stale predecessor cannot become authority merely by retaining
            // its worker handle after the durable successor is selected.
            assert!(validate_durable_task_execution(&parent_id,&task_id,7,&lease).is_err());
        }
        let worker_task = task_id.clone();
        let worker_parent = parent_id.clone();
        let worker_cancel = fixture.cancellation.clone();
        task_handles().lock().unwrap().insert(task_id.clone(), AgentTaskHandle {
            control_id: task_id.clone(), lease: Some(fresh_lease.clone()), execution: None,
            permission_resume: Some(resume.clone()), cancellation: worker_cancel.clone(), child_session_id: child_id.clone(),
        });
        let original_input = json!({"path":"fixture-output.txt","content":"test"});
        assert!(wait_for_background_agent_permission(&child_id, "file.read", &original_input,
            &[], &original_core_permission_id).unwrap_err().contains("original Core effect"));
        assert!(wait_for_background_agent_permission(&child_id, "file.write", &json!({"path":"elsewhere"}),
            &[], &original_core_permission_id).unwrap_err().contains("original Core effect"));
        let mut wrong_run = resume.clone();
        let mut continuation: Value = serde_json::from_str(&wrong_run.suspension.continuation_json).unwrap();
        continuation["run_id"] = json!("different-run");
        wrong_run.suspension.continuation_json = continuation.to_string();
        task_handles().lock().unwrap().get_mut(&task_id).unwrap().permission_resume = Some(wrong_run);
        assert!(wait_for_background_agent_permission(&child_id, "file.write", &original_input,
            &[], &original_core_permission_id).unwrap_err().contains("different Run"));
        task_handles().lock().unwrap().get_mut(&task_id).unwrap().permission_resume = Some(resume.clone());
        // A different Core request is not this authorization. With no active
        // execution authority yet it cannot create a fresh permission either.
        assert!(wait_for_background_agent_permission(&child_id, "file.write", &original_input,
            &[], "different-core-permission").unwrap_err().contains("no retained execution authority"));
        fixture.worker = Some(std::thread::spawn(move || {
            let result = validate_cold_permission_execution(&worker_parent, &worker_task, 7, &resume, &fresh_lease, &worker_cancel)
                .and_then(|()| {
                    let execution = crate::runtime_task_port::RuntimeTaskExecutionAuthority::new(fresh_lease);
                    task_handles().lock().unwrap().get_mut(&worker_task).unwrap().execution = Some(execution);
                    wait_for_background_agent_permission(&child_id, "file.write",
                        &json!({"path":"fixture-output.txt","content":"test"}), &[], &original_core_permission_id)
                });
            let _ = sender.send(result);
        }));
    } else { fixture.worker = Some(std::thread::spawn(move || {
        let HostRequest::RequestPermission { capability_id, input, permission_instructions,
            permission_request_id, .. } = core_permission.unwrap().payload else { unreachable!() };
        let result = wait_for_background_agent_permission(&child_id, &capability_id,
            &input, &permission_instructions, &permission_request_id);
        let _ = sender.send(result);
    })); }
    let suspension = wait_for_fact("original suspension", || {
        let fact = store.latest_runtime_task_permission_suspension(&task_id).unwrap();
        if fact.is_none() {
            if let Ok(result) = receiver.try_recv() { panic!("worker ended before suspension: {result:?}"); }
        }
        fact
    });
    let continuation: Value = serde_json::from_str(&suspension.continuation_json).unwrap();
    if mode == 22 {
        // A durable permission occurrence is not delivery authority. Claim the
        // exact pending effect before simulating its Host settlement response.
        let claimed_at = unix_epoch_millis();
        let claimed = application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: child_run.begin.run_id.clone(), worker_generation: original.worker_generation.clone(),
            lease_owner: original.worker_id.clone(), claimed_at_ms: claimed_at,
            lease_expires_at_ms: claimed_at + 60_000, limit: 1,
        }).unwrap();
        let request = claimed.requests.into_iter().next().expect("pending permission effect");
        assert_eq!(request.effect_id, settlement_request.as_ref().unwrap().effect_id);
        let HostRequest::RequestPermission { permission_request_id, .. } = &request.payload else { unreachable!() };
        for (index, payload) in [HostResponse::ExecutionStarted, HostResponse::PermissionResolved {
            request_id: permission_request_id.clone(), resolution: HostPermissionResolution::deny(),
        }].into_iter().enumerate() {
            application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                conversation_id: request.session_id.clone(), run_id: child_run.begin.run_id.clone(),
                invocation_config_digest: child_run.invocation_config_digest.clone(), received_at_ms: unix_epoch_millis(),
                response: HostResponseEnvelope {
                    request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
                    host_id: request.host_id.clone(), lease_id: request.lease_id.clone(),
                    session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                    catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                    attempt_id: request.attempt_id.clone(), delegated_invocation_id: request.delegated_invocation_id.clone(),
                    delivery: request.delivery.clone(), sequence: request.resume_after_sequence.unwrap_or(0) + index as u64 + 1,
                    payload,
                },
            }).unwrap();
        }
        wake_task_waiters();
        assert!(matches!(receiver.recv_timeout(Duration::from_secs(15)).unwrap().unwrap(),
            Some(BackgroundPermissionWait::Settled)));
        let closed = store.permission_request(&suspension.permission_request_id).unwrap().unwrap();
        assert!(closed.decision.is_none(), "a Core receipt is not a fabricated local user decision");
        assert!(closed.resolved_at_ms.is_none());
        assert!(matches!(closed.closure.as_ref().unwrap().source,
            nola_domain::permission_closure::PermissionClosureSource::CoreSettled { .. }));
        assert_eq!(store.latest_runtime_task_permission_suspension(&task_id).unwrap().unwrap().state, "settled");
        assert_eq!(store.runtime_task(&task_id).unwrap().unwrap().status, "queued");
        assert!(authority.active_lease().unwrap().is_none());
        assert!(store.runtime_task_permission_continuation_grant(&task_id, &suspension.id,
            &nola_application::PermissionContinuationGrantSelector::RecoveryCandidate).unwrap().is_none());
        let reopened = nola_store::NolaStore::open(&database).unwrap();
        assert_eq!(reopened.permission_request(&closed.request_id).unwrap().unwrap(), closed);
        assert_eq!(reopened.runtime_task_run_binding(&task_id).unwrap().unwrap().run_id, child_run.begin.run_id);
        return;
    }
    if mode == 21 {
        fixture.cancellation.request();
        wake_task_waiters();
        let outcome = receiver.recv_timeout(Duration::from_secs(15)).unwrap().unwrap();
        assert!(matches!(outcome, Some(BackgroundPermissionWait::StopRequested)));
        let permission = store.permission_request(&suspension.permission_request_id).unwrap().unwrap();
        assert!(permission.decision.is_none());
        assert!(permission.resolved_at_ms.is_none());
        assert!(store.runtime_task(&task_id).unwrap().unwrap().cancel_requested);
        assert!(authority.active_lease().unwrap().is_none());
        assert!(store.runtime_task_permission_continuation_grant(&task_id, &suspension.id,
            &nola_application::PermissionContinuationGrantSelector::RecoveryCandidate).unwrap().is_none());
        // Follow the actual supervisor path after the waiting callback exits.
        // StopRequested is not a terminal result; only the canonical child
        // cancellation may close the wait and seal the Task.
        fixture.worker.take().unwrap().join().unwrap();
        let waiting = store.runtime_task(&task_id).unwrap().unwrap();
        let mut config = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
        config.tool_settings.project_root = waiting.project_root.clone().into();
        config.session.session_id = waiting.owner_session_id.clone();
        recover_orphaned_agent_tasks(&config);
        let receipt = store.runtime_task_terminal_receipt(&task_id).unwrap().unwrap();
        assert_eq!(receipt.status, "sealed");
        assert!(matches!(&receipt.journal_result.as_ref().unwrap().seal.result.value,
            nola_domain::run_result::RunResultValue::Cancelled { cancellation_id, .. }
            if cancellation_id == &format!("task-user-input-cancel:{task_id}")));
        let closed = store.permission_request(&suspension.permission_request_id).unwrap().unwrap();
        assert_eq!(closed.status, "withdrawn");
        assert!(closed.decision.is_none());
        assert!(closed.resolved_at_ms.is_none());
        let source: nola_domain::permission_closure::CorePermissionSourceProof =
            serde_json::from_value(continuation["core_permission_source"].clone()).unwrap();
        assert!(matches!(&closed.closure.as_ref().unwrap().source,
            nola_domain::permission_closure::PermissionClosureSource::RunSealed {
                permission, seal_digest, ..
            } if permission == &source.permission
                && seal_digest == &receipt.journal_result.as_ref().unwrap().seal.seal_digest));
        assert!(store.pending_permission_for_task(&task_id).unwrap().is_none());
        drop(store);
        let reopened = nola_store::NolaStore::open(&database).unwrap();
        recover_orphaned_agent_tasks(&config);
        assert_eq!(reopened.permission_request(&closed.request_id).unwrap().unwrap(), closed);
        assert_eq!(reopened.runtime_task_terminal_receipt(&task_id).unwrap().unwrap(), receipt);
        assert!(matches!(crate::permission_request_port::authorize_permission(&closed.request_id,
            nola_application::PermissionAuthorizationDecision::AllowOnce, unix_epoch_millis()).unwrap(),
            nola_application::PermissionResolutionOutcome::Closed { .. }));
        let inspect = rusqlite::Connection::open(&database).unwrap();
        assert_eq!(inspect.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM runtime_task_attempts WHERE task_id=?1",
            [&task_id], |row| row.get(0)).unwrap(), 1, "cancellation must not start another worker");
        assert_eq!(inspect.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM runs WHERE conversation_id=?1",
            [waiting.child_session_id.as_deref().unwrap()], |row| row.get(0)).unwrap(), 1,
            "cancellation must not start a replacement Run");
        return;
    }
    let proof: nola_domain::permission_closure::CorePermissionSourceProof =
        serde_json::from_value(continuation["core_permission_source"].clone()).unwrap();
    assert_eq!(proof.permission.run_id, child_run.begin.run_id);
    if let Some(source_id) = emitted_permission_id {
        let persisted = store.permission_request(&suspension.permission_request_id).unwrap().unwrap();
        assert_eq!(persisted.core_permission_request_id.as_deref(), Some(source_id.as_str()));
        assert_eq!(continuation["core_permission_request_id"], source_id);
        assert_eq!(persisted.capability, "file.write");
        assert_eq!(serde_json::from_str::<Value>(&persisted.input_json).unwrap(),
            json!({"path":"fixture-output.txt","content":"test"}));
    }
    let observed_notification = if !cold {
        // Independently consumed model mailbox must not hide the durable
        // permission occurrence from the UI consumer.
        rusqlite::Connection::open(&database).unwrap().execute(
            "UPDATE durable_tasks SET unread=0 WHERE id=?1", [&task_id]).unwrap();
        let task = store.runtime_task(&task_id).unwrap().unwrap();
        let mut presentation = crate::host_invocation_request::HostInvocationRequest::for_test().unwrap();
        presentation.session.save_session = true;
        presentation.tool_settings.project_root = std::path::PathBuf::from(&task.project_root);
        presentation.session.session_id = task.owner_session_id.clone();
        if mode == 0 {
            let cancellation = RuntimeExecutionControl::default();
            let context = ToolContext {
                memory_binding: nola_domain::memory_owner::MemoryExecutionBinding::local_profile(),
                language_services: Default::default(),
                output_store: tools::ToolOutputStore::for_test(),
                skills: crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
                allowed_dirs: vec![presentation.tool_settings.project_root.clone()],
                policy: tools::ToolExecutionPolicy::new(Vec::new(), false, "ask".into(), Vec::new()),
                permission_scope: Default::default(), cancellation: cancellation.clone(),
                plan_state: Default::default(),
                browser: tools::BrowserRuntime::new(fixture._root.path().join("browser")),
            };
            let args = json!({"task_id":task_id,"timeout_sec":1}).as_object().unwrap().clone();
            let started = Instant::now();
            let outcome = wait_task(&presentation, &args, &context);
            assert!(started.elapsed() >= Duration::from_secs(1), "approval wait must not immediately spin the model");
            assert!(!outcome.error, "{}", outcome.text);
            let value: Value = serde_json::from_str(&outcome.text).unwrap();
            assert_eq!(value["action_required"], "permission");
            assert_eq!(value["wait_timed_out"], true);
            cancellation.request();
            let cancelled = wait_task(&presentation, &args, &context);
            assert!(cancelled.error && cancelled.text.contains("Task wait cancelled"));
            let pending = store.permission_request(&suspension.permission_request_id).unwrap().unwrap();
            assert_eq!(pending.status, "pending");
            assert!(pending.decision.is_none(), "waiting cannot grant or deny permission");
            assert!(!store.runtime_task(&task_id).unwrap().unwrap().cancel_requested,
                "cancelling the observer cannot cancel its child");
        }
        let notifications = take_runtime_task_notifications(&presentation).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].delivery.occurrence_id, format!("permission:{}", suspension.permission_request_id));
        assert_eq!(acknowledge_runtime_task_notification_delivery(&notifications[0]).unwrap(), nola_application::RuntimeTaskNotificationAcknowledgement::Delivered);
        assert!(take_runtime_task_notifications(&presentation).unwrap().is_empty());
        Some(notifications[0].clone())
    } else { None };
    assert_eq!(continuation["run_id"], child_run.begin.run_id);
    assert!(authority.with_active::<()>(|_| panic!("suspended work executed")).unwrap().is_none());
    assert!(authority.with_renewal::<()>(|_| panic!("original lease renewed during permission wait")).unwrap().is_none());
    assert!(!store.retry_runtime_task(&nola_application::RuntimeTaskLeaseWitness::from(&original),
        now + 60_000, "stale worker", unix_epoch_millis()).unwrap());
    if mode == 0 {
        let displayed = permission_for_parent_task(&parent_id, &parent_turn, &task_id)
            .unwrap().expect("real child permission is observable by its parent");
        assert!(permission_for_parent_task("foreign-parent", &parent_turn, &task_id).unwrap().is_none());
        assert!(permission_for_parent_task(&parent_id, "foreign-turn", &task_id).unwrap().is_none());
        let mut changed = displayed.clone();
        changed.source.arguments = json!({"path":"other.txt"});
        assert!(resolve_parent_task_permission(&changed, PermissionChoice::Once).is_err());
        assert_eq!(store.permission_request(&suspension.permission_request_id).unwrap().unwrap().status, "pending");
        resolve_parent_task_permission(&displayed, PermissionChoice::Once).unwrap();
        assert!(resolve_parent_task_permission(&displayed, PermissionChoice::Once).is_err());
    }
    if !cold && mode != 0 { assert!(matches!(crate::permission_request_port::authorize_permission(&suspension.permission_request_id,
        nola_application::PermissionAuthorizationDecision::AllowOnce, unix_epoch_millis()).unwrap(),
        nola_application::PermissionResolutionOutcome::Resolved { .. })); }
    if let Some(mut notification) = observed_notification {
        // The task may resume between drawing an observed permission and its
        // display ACK. A distinct consumer proves this is not an existing ACK.
        notification.delivery.consumer_id = "delayed-renderer".into();
        notification.delivery.command_id = format!("delayed-display:{}", suspension.permission_request_id);
        assert_eq!(deliver_runtime_task_notification(&notification, || Ok(crate::terminal_ui::HistoryPresentation::Presented)).unwrap(),
            Some(nola_application::RuntimeTaskNotificationAcknowledgement::Superseded));
        assert!(!crate::runtime_task_port::notification_delivered(&notification.delivery).unwrap());
        assert_eq!(store.permission_request(&suspension.permission_request_id).unwrap().unwrap().decision.as_deref(), Some("allow_once"));
    }
    let receipt = wait_for_fact("continuation grant", || {
        let fact = store.runtime_task_permission_continuation_grant(&task_id, &suspension.id,
            &nola_application::PermissionContinuationGrantSelector::RecoveryCandidate).unwrap();
        if fact.is_none() {
            if let Ok(result) = receiver.try_recv() { panic!("worker ended before continuation grant: {result:?}"); }
        }
        fact
    });
    assert_ne!(receipt.granted_attempt.lease_token, original.lease_token);
    assert_ne!(receipt.granted_attempt.worker_generation, original.worker_generation);
    if cold {
        assert!(authority.active_lease().unwrap().is_none(), "cold origin remains revoked");
    } else if let Some(active) = authority.active_lease().unwrap() {
        // Activation may race the observer after the durable grant commits;
        // it may only select that exact successor, never the suspended lease.
        let active=active.snapshot().unwrap();
        assert_eq!(active.id,receipt.granted_attempt.id);
        assert_eq!(active.task_id,receipt.granted_attempt.task_id);
        assert_eq!(active.worker_id,receipt.granted_attempt.worker_id);
        assert_eq!(active.worker_generation,receipt.granted_attempt.worker_generation);
        assert_eq!(active.lease_token,receipt.granted_attempt.lease_token);
    }
    if !cold {
        let count:i64=rusqlite::Connection::open(&database).unwrap().query_row(
            "SELECT COUNT(*) FROM agent_commands WHERE session_id=?1",[&parent_id],|row|row.get(0)).unwrap();
        assert_eq!(count,0,"a committed continuation grants execution without parent lease handoff");
    }
    // The worker consumes its exact durable continuation, without any parent
    // permission FIFO or mirrored Core lease becoming execution authority.
    let decision = receiver.recv_timeout(Duration::from_secs(15))
        .unwrap_or_else(|error| panic!("mode {mode}: worker continuation did not settle: {error}"))
        .unwrap_or_else(|error| panic!("mode {mode}: worker continuation failed: {error}"));
    assert!(matches!(decision, Some(BackgroundPermissionWait::Decision(BackgroundPermissionDecision::Allow))));
    let active = task_handles().lock().unwrap().get(&task_id).unwrap().execution.as_ref().unwrap()
        .active_lease().unwrap().unwrap().snapshot().unwrap();
    assert_eq!(active.id, receipt.granted_attempt.id);
    assert_eq!(active.task_id,receipt.granted_attempt.task_id);
    assert_eq!(active.worker_id,receipt.granted_attempt.worker_id);
    assert_eq!(active.worker_generation,receipt.granted_attempt.worker_generation);
    assert_eq!(active.lease_token,receipt.granted_attempt.lease_token);
    let current:Option<nola_application::RuntimeTaskAttempt>=application.execute(nola_application::ApplicationCommand::RuntimeTask {
        command:nola_application::RuntimeTaskCommand::CurrentAttempt {task_id:task_id.clone()},
    }).unwrap();
    assert_eq!(current.as_ref(),Some(&active),"resumed execution must equal the actual current Store attempt");
    assert_eq!(lease.snapshot().unwrap(), original);
    let projection = crate::session::load_session_task_projection(&parent_id).unwrap().unwrap();
    let parent = projection.tasks.get(&task_id).unwrap();
    assert_eq!(parent.state, nola_core::agent::TaskState::Queued);
    assert!(parent.durability.ownership_token.is_none(),"parent projection never owns the successor worker lease");
    assert_eq!(store.runtime_task_permission_continuation_grant(&task_id, &suspension.id,
        &nola_application::PermissionContinuationGrantSelector::ExactAttempt { attempt_id: receipt.granted_attempt.id.clone() }).unwrap().unwrap(), receipt);
    let connection = rusqlite::Connection::open(&database).unwrap();
    let unacked: i64 = connection.query_row("SELECT COUNT(*) FROM agent_commands WHERE session_id=?1",
        [&parent_id], |row| row.get(0)).unwrap();
    assert_eq!(unacked, 0, "permission continuation must not manufacture parent lease handoffs");
    let request_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM permission_requests WHERE task_id=?1", [&task_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(request_count, 1, "recovery must reuse the original permission occurrence");
}
