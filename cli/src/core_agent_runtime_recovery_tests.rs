// SPDX-License-Identifier: Apache-2.0
// Real CLI Host/provider/application entrypoint regression. cfg(test) Application
// uses the SQLite in-process composition; this does not claim Service IPC coverage.
#[cfg(test)]
pub(crate) fn fixture_completion_audit(input: &[u8]) -> serde_json::Value {
    let request: serde_json::Value = serde_json::from_slice(input).unwrap();
    let receipts: serde_json::Value = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|message| message["content"].as_str())
        .find_map(|content| {
            content.split_once("Current-turn Host-confirmed successful mutation call IDs: ")
        })
        .map(|(_, rest)| {
            serde_json::from_str(rest.split_once(". IDs establish").unwrap().0).unwrap()
        })
        .expect(
            "audit fixture must cite the canonical Host receipt list, not invented provider IDs",
        );
    let outcome = if receipts.as_array().unwrap().is_empty() {
        "answered"
    } else {
        "action_completed"
    };
    let report = serde_json::json!({"decision":"accept", "outcome":outcome, "effect_call_ids":receipts,
        "constraints":[], "items":[{"label":"Fixture task requirement", "eligibility":"eligible",
        "evidence":"Observed fixture evidence supports the candidate"}], "violations":[]});
    serde_json::json!({"content":report.to_string()})
}

#[cfg(test)]
mod permission_recovery_entrypoint_tests {
    use super::*;
    fn mac_launch_fixture(config: &HostInvocationRequest) -> nola_host_protocol::HostMacLaunchDescriptor {
        // The Mac transport descriptor is a wire fixture, independent of the
        // native temporary directory used by this Host execution fixture.
        let project = if cfg!(windows) { std::path::PathBuf::from("/fixture/mac-workspace") } else { config.tool_settings.project_root.clone() };
        let directories = if cfg!(windows) { vec![project.clone()] } else { config.tool_settings.allowed_dirs.clone() };
        let descriptor: nola_host_protocol::HostMacLaunchDescriptor = serde_json::from_value(serde_json::json!({
            "version": 1, "input_origin": "interactive_user", "project_path": project,
            "authorized_workspace_paths": directories,
            "allowed_directory_arguments": directories.iter().flat_map(|path| ["--allow-dir".to_string(), path.to_string_lossy().into_owned()]).collect::<Vec<_>>(),
            "skill_directory_arguments": ["--no-skills"], "active_skill_ids": [],
            "model_profile_id": "fixture-profile", "model_profile_revision":null, "fallback_profile_ids": [],
            "permission_mode": "cautious", "memory_enabled": false,
            "context_window_tokens": null, "max_iterations": config.execution.max_iterations,
            "model_capabilities": {}, "vision_input": false,
            "permission_rules": config.runtime.core_permission_rules, "host_attachments": []
        })).unwrap();
        descriptor.validate().unwrap();
        descriptor
    }
    fn http_route(
        route: &nola_host_protocol::HostModelRouteBinding,
    ) -> &nola_host_protocol::HostHttpModelRouteBinding {
        match route {
            nola_host_protocol::HostModelRouteBinding::Http(http) => http,
            _ => panic!("HTTP fixture"),
        }
    }
    use std::io::{BufRead, Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Fixture {
        root: std::path::PathBuf,
        session: String,
        environment: Vec<(&'static str, Option<std::ffi::OsString>)>,
        stop: Arc<AtomicBool>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = invalidate_core_agent_session(&self.session);
            for (key, value) in &self.environment {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn assert_original_permission_trace(
        request: &crate::tools::ToolRequest,
        calls: &Mutex<Vec<String>>,
    ) {
        let session_id =
            crate::tools::current_host_session_id().expect("permission fixture session");
        let trace = crate::tools::take_pending_host_permission_trace(
            &session_id,
            request,
            &serde_json::Value::Object(request.args.clone()),
            "fixture permission retry",
        )
        .unwrap_or_else(|| {
            panic!(
                "missing canonical trace after {} prior presentations",
                calls.lock().unwrap().len()
            )
        });
        assert_eq!(trace["kind"], "tool.permission.requested");
        let call_id = trace["payload"]["call_id"]
            .as_str()
            .expect("canonical call identity")
            .to_string();
        let mut calls = calls.lock().unwrap();
        if let Some(original) = calls.first() {
            assert_eq!(&call_id, original);
        }
        calls.push(call_id);
        assert_eq!(trace["payload"]["permission_request_id"], request.id);
    }

    fn binding_config() -> (Fixture, HostInvocationRequest) {
        let root = std::env::temp_dir().join(format!("nola-binding-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let fixture = Fixture {
            root: root.clone(),
            session: format!("binding-{}", uuid::Uuid::new_v4()),
            environment: ["NOLA_HOME", "NOLA_DB_PATH"]
                .into_iter()
                .map(|k| (k, std::env::var_os(k)))
                .collect(),
            stop: Arc::new(AtomicBool::new(false)),
        };
        std::env::set_var("NOLA_HOME", &root);
        std::env::set_var("NOLA_DB_PATH", root.join("never-opened.sqlite"));
        let mut config = crate::config::Config::from_args(vec![
            "nola".into(),
            "--project".into(),
            root.to_string_lossy().into_owned(),
            "--endpoint".into(),
            "https://provider.invalid/v1?token=query-secret".into(),
            "--api-key".into(),
            "private-api-key".into(),
            "--model".into(),
            "fixture-model".into(),
        ])
        .and_then(|config| config.host_invocation_request())
        .unwrap();
        freeze_host_retry_admission(&mut config);
        freeze_host_memory_admission(&mut config).unwrap();
        (fixture, config)
    }

    // These full Host/replay fixtures retain several debug Core states at once.
    // Match the bounded stack used by the existing Goal binding fixtures; this
    // is test-thread allocation only, not a production execution policy.
    fn run_full_host_recovery_fixture(test: fn()) {
        std::thread::Builder::new().name("host-recovery-fixture".into())
            .stack_size(16 * 1024 * 1024).spawn(test).unwrap().join().unwrap();
    }

    #[test]
    fn shared_goal_entry_never_steals_a_live_lease_or_starts_a_second_round() { run_full_host_recovery_fixture(shared_goal_entry_never_steals_a_live_lease_or_starts_a_second_round_inner); }
    fn shared_goal_entry_never_steals_a_live_lease_or_starts_a_second_round_inner() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.tools_enabled = false;
        config.tool_settings.skill_dirs = Some(Vec::new());
        crate::core_agent_runtime::submit_agent_core_goal_command(&config,
            nola_application::goal_coordinator::create_goal_command("owned-start","owned goal","",crate::goal_runtime::goal_now_epoch_ms())).unwrap();
        let mut application=crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let original=application.active_run(&fixture.session).unwrap().unwrap().run_id;
        let active=application.active_run(&fixture.session).unwrap().unwrap();
        application.reduce_and_commit_active_submission(nola_application::ReduceAndCommitActiveSubmission {
            conversation_id:fixture.session.clone(),run_id:active.run_id,
            invocation_config_digest:active.invocation_config_digest.unwrap(),transition_id:"fixture-live-lease".into(),
            submission:AgentSubmission::GoalRuntime {envelope:nola_core::agent::GoalRuntimeCommandEnvelope {
                operation_id:"owned-lease".into(),command:nola_core::agent::GoalRuntimeCommand::Claim {
                    owner:"existing-host".into(),lease_until_epoch_ms:crate::goal_runtime::goal_now_epoch_ms()+60_000,
                    now_epoch_ms:crate::goal_runtime::goal_now_epoch_ms()}}},committed_at_ms:crate::goal_runtime::goal_now_epoch_ms() }).unwrap();
        let before = crate::session::load_session_goal_projection(&fixture.session).unwrap().unwrap();
        let mut output = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        let error = crate::goal_runtime::run_host_goal(&mut config, "", "owned goal", "", &mut output,
            RuntimeExecutionControl::default()).unwrap_err();
        assert!(error.to_ascii_lowercase().contains("lease"), "{error}");
        let after = crate::session::load_session_goal_projection(&fixture.session).unwrap().unwrap();
        assert_eq!(before, after);
        let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        assert_eq!(app.active_run(&fixture.session).unwrap().unwrap().run_id,original);
    }

    #[test]
    fn shared_goal_entry_yields_and_cold_resumes_exact_permission_without_new_round() { run_full_host_recovery_fixture(shared_goal_entry_yields_and_cold_resumes_exact_permission_without_new_round_inner); }
    fn shared_goal_entry_yields_and_cold_resumes_exact_permission_without_new_round_inner() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        config.model_settings.endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = true;
        config.tool_settings.auto_approve = false;
        config.tool_settings.tool_allowlist = Some(vec!["write_file".into()]);
        config.tool_settings.allowed_dirs = vec![fixture.root.clone()];
        config.runtime.core_permission_rules.push(CorePermissionRule {
            capability_pattern: nola_core::agent::tool_permission_domain("write_file").unwrap(),
            resource_pattern: "*".into(), decision: CoreRuleDecision::Ask,
            source: PermissionRuleSource::UserGlobal,
        });
        config.runtime.mac_launch = Some(mac_launch_fixture(&config));
        let target = fixture.root.join("goal-permission.txt");
        let server_target = target.clone();
        let posts = Arc::new(AtomicUsize::new(0));
        let count = posts.clone();
        let stop = fixture.stop.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5)); continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new(); reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_string();
                let mut length = 0;
                loop {
                    let mut header = String::new(); reader.read_line(&mut header).unwrap();
                    if header == "\r\n" { break; }
                    if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") { length = v.trim().parse::<usize>().unwrap(); }
                }
                let mut input = vec![0; length]; reader.read_exact(&mut input).unwrap();
                let (mime, body) = if path == "/v1/models" {
                    ("application/json", serde_json::json!({"data":[{"id":"fixture-model"}]}).to_string())
                } else {
                    assert_eq!(path, "/v1/chat/completions");
                    assert_eq!(count.fetch_add(1, Ordering::SeqCst), 0, "permission recovery must not request another model turn");
                    let value: Value = serde_json::from_slice(&input).unwrap();
                    assert!(value.to_string().contains("Autonomous goal mode"));
                    let delta = serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"goal-write","type":"function","function":{"name":"write_file","arguments":serde_json::json!({"path":server_target,"content":"requires approval"}).to_string()}}]},"finish_reason":null}]});
                    let end = serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
                    ("text/event-stream", format!("data: {delta}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
                };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        let _session = crate::tools::enter_host_permission_session(&fixture.session);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let observed = calls.clone();
        let _resolver = crate::tools::enter_host_permission_resolver(Arc::new(move |request, _, _| {
            assert_original_permission_trace(request, &observed);
            ToolPermissionResolution::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Disconnected)
        }));
        let mut output = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        let first = crate::goal_runtime::run_host_goal(&mut config, "write the file", "Write the approved file", "", &mut output, RuntimeExecutionControl::default()).unwrap();
        assert!(matches!(first, crate::goal_runtime::GoalCycleOutcome::Yielded(_)));
        let goal = crate::session::load_session_goal_projection(&fixture.session).unwrap().unwrap().active.unwrap();
        assert_eq!(goal.status, nola_core::agent::GoalStatus::NeedsUser);
        assert_eq!(goal.contracts.len(), 1);
        assert_eq!(goal.contracts[0].status, nola_core::agent::GoalContractStatus::Executing);
        assert!(goal.lease.is_none());
        let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let original_run = app.active_run(&fixture.session).unwrap().unwrap().run_id;
        invalidate_core_agent_session(&fixture.session).unwrap();
        // Reopening must use the frozen Run, not changed Host defaults.
        config.model_settings.endpoint = "http://127.0.0.1:9/changed-default".into();
        let resumed = crate::goal_runtime::run_host_goal(&mut config, "", "Write the approved file", "", &mut output, RuntimeExecutionControl::default()).unwrap();
        assert!(matches!(resumed, crate::goal_runtime::GoalCycleOutcome::Yielded(_)));
        let after = crate::session::load_session_goal_projection(&fixture.session).unwrap().unwrap().active.unwrap();
        assert_eq!(after.contracts, goal.contracts, "a permission yield must retain the exact executing contract");
        assert_eq!(app.active_run(&fixture.session).unwrap().unwrap().run_id, original_run);
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], calls[1]);
        assert!(!target.exists());
        fixture.stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
    }

    #[test]
    fn repeated_host_recovery_never_rebinds_a_prepared_or_live_model_attempt() {
        use nola_core::agent::host::{HostResponse, HostResponseEnvelope};
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        let (_, run_id) = stage_registered_core_agent_turn_without_dispatch(&config, "retain the pending model").unwrap();
        let mut application = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        for started in [false, true] {
            if started {
                let now = nola_runtime_driver::monotonic_epoch_millis();
                let request = application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
                    run_id: run_id.clone(), worker_generation: "live-worker".into(), lease_owner: "live-owner".into(),
                    claimed_at_ms: now, lease_expires_at_ms: now + 60_000, limit: 1,
                }).unwrap().requests.into_iter().next().unwrap();
                let digest = application.active_run(&fixture.session).unwrap().unwrap().invocation_config_digest.unwrap();
                application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                    conversation_id: fixture.session.clone(), run_id: run_id.clone(), invocation_config_digest: digest,
                    received_at_ms: now, response: HostResponseEnvelope {
                        request_id: request.request_id, attempt_id: request.attempt_id,
                        delegated_invocation_id: request.delegated_invocation_id, effect_id: request.effect_id,
                        host_id: request.host_id, lease_id: request.lease_id, session_id: request.session_id,
                        turn_id: request.turn_id, catalog_revision: request.catalog_revision,
                        availability_epoch: request.availability_epoch, delivery: request.delivery,
                        sequence: request.resume_after_sequence.map_or(0, |sequence| sequence + 1),
                        payload: HostResponse::ExecutionStarted,
                    },
                }).unwrap();
            }
            let before = application.load_committed_host_requests(&run_id).unwrap();
            let frontier = application.active_run(&fixture.session).unwrap().unwrap().next_event_sequence;
            let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
            let mut host = host.lock().unwrap();
            assert!(host.runtime.execution.pending_effects.iter().any(|effect| effect.capability_id == "model.stream"));
            // A retained/cold Host hint is never permission to rotate an active
            // physical attempt. The Core ledger alone owns that decision.
            host.resume_pending_once = true;
            let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
            for _ in 0..10 { host.prepare_pending_effect_recovery(&mut emitter).unwrap(); }
            assert_eq!(serde_json::to_value(application.load_committed_host_requests(&run_id).unwrap().requests).unwrap(),
                serde_json::to_value(before.requests).unwrap());
            assert_eq!(application.active_run(&fixture.session).unwrap().unwrap().next_event_sequence, frontier,
                "polling recovery must not append Resume events");
        }
    }

    #[test]
    fn scheduled_expired_model_delivery_recovers_rebinds_and_becomes_claimable() {
        use nola_core::agent::host::{HostResponse, HostResponseEnvelope};
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        let (_, run) = stage_registered_core_agent_turn_without_dispatch(&config, "expired model fixture").unwrap();
        let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let original = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: "expiry-test-worker".into(), lease_owner: "expiry-test-owner".into(),
            claimed_at_ms: now, lease_expires_at_ms: now + 100, limit: 1,
        }).unwrap().requests.into_iter().next().unwrap();
        let digest = app.active_run(&fixture.session).unwrap().unwrap().invocation_config_digest.unwrap();
        app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest,
            received_at_ms: now, response: HostResponseEnvelope {
                request_id: original.request_id.clone(), attempt_id: original.attempt_id.clone(),
                delegated_invocation_id: original.delegated_invocation_id.clone(), effect_id: original.effect_id.clone(),
                host_id: original.host_id.clone(), lease_id: original.lease_id.clone(), session_id: original.session_id.clone(),
                turn_id: original.turn_id.clone(), catalog_revision: original.catalog_revision,
                availability_epoch: original.availability_epoch, delivery: original.delivery.clone(),
                sequence: original.resume_after_sequence.map_or(0, |n| n + 1), payload: HostResponse::ExecutionStarted,
            },
        }).unwrap();
        // A real expired lease, not a rewritten Store or forged death proof.
        while nola_runtime_driver::monotonic_epoch_millis() <= now + 100 {
            std::thread::sleep(Duration::from_millis(5));
        }
        let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
        let mut host = host.lock().unwrap();
        recover_scheduled_run_deliveries(&mut host, &config,
            Some(&nola_protocol::RuntimeEffectWorkReason::RecoverExpiredLease)).unwrap();
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        host.prepare_pending_effect_recovery(&mut emitter).unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let successor = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run, worker_generation: "successor-worker".into(), lease_owner: "successor-owner".into(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 1,
        }).unwrap().requests.into_iter().next().expect("expired work must no longer loop on an empty claim");
        assert_eq!(successor.effect_id, original.effect_id);
        assert_ne!(successor.request_id, original.request_id);
        assert_ne!(successor.delivery.unwrap().attempt_id, original.delivery.unwrap().attempt_id);
    }

    #[test]
    fn retained_stop_requires_exact_current_delivery_without_prior_attempts() {
        use nola_core::agent::host::{HostResponse, HostResponseEnvelope};
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        let (_, run) = stage_registered_core_agent_turn_without_dispatch(&config, "expired model fixture").unwrap();
        let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let worker = crate::process_observation::current_worker_identity().unwrap().lease_identity().unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let original = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 1,
        }).unwrap().requests.into_iter().next().unwrap();
        let digest = app.active_run(&fixture.session).unwrap().unwrap().invocation_config_digest.unwrap();
        app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest,
            received_at_ms: now, response: HostResponseEnvelope {
                request_id: original.request_id.clone(), attempt_id: original.attempt_id.clone(),
                delegated_invocation_id: original.delegated_invocation_id.clone(), effect_id: original.effect_id.clone(),
                host_id: original.host_id.clone(), lease_id: original.lease_id.clone(), session_id: original.session_id.clone(),
                turn_id: original.turn_id.clone(), catalog_revision: original.catalog_revision,
                availability_epoch: original.availability_epoch, delivery: original.delivery.clone(),
                sequence: original.resume_after_sequence.map_or(0, |n| n + 1), payload: HostResponse::ExecutionStarted,
            },
        }).unwrap();
        let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        host.lock().unwrap().commit_core_submission("cancel-current-target", AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "cancel-current-target".into(), reason: Some("fixture".into()),
        }, &mut emitter).unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let controls = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 64,
        }).unwrap();
        let control = controls.requests.iter().find(|request| matches!(&request.payload,
            nola_core::agent::host::HostRequest::Cancel { target_effect_id, .. } if target_effect_id == &original.effect_id)).unwrap();
        validate_local_cancellation_stop(&fixture.session, &run, control,
            &original.request_id, original.delivery.as_ref()).unwrap();
        for field in ["generation", "owner", "token", "attempt", "expiry"] {
            let mut changed = original.delivery.clone().unwrap();
            match field {
                "generation" => changed.worker_generation.push_str("-foreign"),
                "owner" => changed.lease_owner.push_str("-foreign"),
                "attempt" => changed.attempt_id.push_str("-foreign"),
                "expiry" => changed.lease_expires_at_ms += 1,
                _ => changed.lease_token.push_str("-foreign"),
            }
            let error = validate_local_cancellation_stop(&fixture.session, &run, control,
                &original.request_id, Some(&changed)).unwrap_err();
            assert!(error.contains("current target delivery fence"), "{field}: {error}");
        }
    }

    include!("core_agent_runtime_stop_process_tests.rs");

    #[test]
    fn external_retained_stop_accepts_current_fence_but_not_process_death() { run_full_host_recovery_fixture(external_retained_stop_accepts_current_fence_but_not_process_death_inner); }
    fn external_retained_stop_accepts_current_fence_but_not_process_death_inner() {
        use nola_core::agent::host::{HostResponse, HostResponseEnvelope};
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = true;
        config.tool_settings.auto_approve = true;
        config.tool_settings.permission_profile = Some(crate::permissions::PermissionProfile::AllowAll);
        config.tool_settings.tool_allowlist = Some(vec!["read_file".into()]);
        config.tool_settings.force_tool_set = "all".into();
        std::fs::write(fixture.root.join("fixture.txt"), "isolated external stop fixture").unwrap();
        let (_, run) = stage_registered_core_agent_turn_without_dispatch(&config, "expired model fixture").unwrap();
        let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let worker = crate::process_observation::current_worker_identity().unwrap().lease_identity().unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let original = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 1,
        }).unwrap().requests.into_iter().next().unwrap();
        let digest = app.active_run(&fixture.session).unwrap().unwrap().invocation_config_digest.unwrap();
        app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest,
            received_at_ms: now, response: HostResponseEnvelope {
                request_id: original.request_id.clone(), attempt_id: original.attempt_id.clone(),
                delegated_invocation_id: original.delegated_invocation_id.clone(), effect_id: original.effect_id.clone(),
                host_id: original.host_id.clone(), lease_id: original.lease_id.clone(), session_id: original.session_id.clone(),
                turn_id: original.turn_id.clone(), catalog_revision: original.catalog_revision,
                availability_epoch: original.availability_epoch, delivery: original.delivery.clone(),
                sequence: original.resume_after_sequence.map_or(0, |n| n + 1), payload: HostResponse::ExecutionStarted,
            },
        }).unwrap();
        let digest = app.active_run(&fixture.session).unwrap().unwrap().invocation_config_digest.unwrap();
        for (sequence, payload) in [
            (1, HostResponse::ModelAttemptStarted { provider: "fixture".into(), model: "fixture-model".into(), reason: "initial".into() }),
            (2, HostResponse::ModelCompleted { items: vec![], tool_calls: vec![serde_json::json!({
                "call_id":"external-read", "name":"read_file", "arguments":{"path":fixture.root.join("fixture.txt")}
            })], usage: serde_json::json!({}) }),
        ] {
            app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest.clone(),
                received_at_ms: now + sequence as i64, response: HostResponseEnvelope {
                    request_id: original.request_id.clone(), attempt_id: original.attempt_id.clone(),
                    delegated_invocation_id: original.delegated_invocation_id.clone(), effect_id: original.effect_id.clone(),
                    host_id: original.host_id.clone(), lease_id: original.lease_id.clone(), session_id: original.session_id.clone(),
                    turn_id: original.turn_id.clone(), catalog_revision: original.catalog_revision,
                    availability_epoch: original.availability_epoch, delivery: original.delivery.clone(), sequence, payload,
                },
            }).unwrap();
        }
        let original = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now + 3, lease_expires_at_ms: now + 60000, limit: 1,
        }).unwrap().requests.into_iter().next().unwrap();
        assert!(matches!(original.payload, nola_core::agent::host::HostRequest::ResolvePermissionResources { .. }),
            "expected canonical resource resolution, got {:?}", original.payload);
        let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
        nola_core::agent::host::AgentHostDriver::dispatch(host.lock().unwrap().runtime.driver_mut(), original,
            &mut |response| {
                app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                    conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest.clone(),
                    received_at_ms: nola_runtime_driver::monotonic_epoch_millis(), response,
                }).map(|_| ())
            }).unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let original = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 1,
        }).unwrap().requests.into_iter().next().expect("resolved resource must produce tool execution");
        assert!(matches!(original.payload, nola_core::agent::host::HostRequest::ExecuteTool { .. }), "unexpected Core request: {:?}", original.payload);
        app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest,
            received_at_ms: now + 4, response: HostResponseEnvelope {
                request_id: original.request_id.clone(), attempt_id: original.attempt_id.clone(),
                delegated_invocation_id: original.delegated_invocation_id.clone(), effect_id: original.effect_id.clone(),
                host_id: original.host_id.clone(), lease_id: original.lease_id.clone(), session_id: original.session_id.clone(),
                turn_id: original.turn_id.clone(), catalog_revision: original.catalog_revision,
                availability_epoch: original.availability_epoch, delivery: original.delivery.clone(),
                sequence: original.resume_after_sequence.map_or(0, |n| n + 1), payload: HostResponse::ExecutionStarted,
            },
        }).unwrap();
        let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        host.lock().unwrap().commit_core_submission("cancel-current-target", AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "cancel-current-target".into(), reason: Some("fixture".into()),
        }, &mut emitter).unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let controls = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 64,
        }).unwrap();
        let control = controls.requests.iter().find(|request| matches!(&request.payload,
            nola_core::agent::host::HostRequest::Cancel { target_effect_id, .. } if target_effect_id == &original.effect_id)).unwrap();
        validate_local_cancellation_stop(&fixture.session, &run, control,
            &original.request_id, original.delivery.as_ref()).unwrap();
        let active = app.active_run(&fixture.session).unwrap().unwrap();
        let error = app.execute::<nola_application::cancellation_process_observation::CancellationProcessObservationPlan>(
            nola_application::ApplicationCommand::InspectCancellationProcessOwners {
                identity: nola_application::cancellation_delivery::CancellationRunIdentity {
                    conversation_id: fixture.session.clone(), expected_run_id: run.clone(),
                    assistant_message_id: active.assistant_message_id.unwrap(),
                }, control: control.clone(),
            }).unwrap_err();
        assert!(error.contains("External target requires independent cancellation reconciliation"), "{error}");
        let mut foreign = control.clone();
        foreign.request_id.push_str("-foreign");
        assert!(validate_local_cancellation_stop(&fixture.session, &run, &foreign,
            &original.request_id, original.delivery.as_ref()).is_err());
        for field in ["generation", "owner", "token", "attempt", "expiry"] {
            let mut changed = original.delivery.clone().unwrap();
            match field {
                "generation" => changed.worker_generation.push_str("-foreign"),
                "owner" => changed.lease_owner.push_str("-foreign"),
                "attempt" => changed.attempt_id.push_str("-foreign"),
                "expiry" => changed.lease_expires_at_ms += 1,
                _ => changed.lease_token.push_str("-foreign"),
            }
            let error = validate_local_cancellation_stop(&fixture.session, &run, control,
                &original.request_id, Some(&changed)).unwrap_err();
            assert!(error.contains("current target delivery fence"), "{field}: {error}");
        }
    }

    #[test]
    fn retained_stop_cannot_confirm_a_core_rebound_delivery() {
        use nola_core::agent::host::{HostResponse, HostResponseEnvelope};
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        let (_, run) = stage_registered_core_agent_turn_without_dispatch(&config, "expired model fixture").unwrap();
        let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let worker = crate::process_observation::current_worker_identity().unwrap().lease_identity().unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let original = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 100, limit: 1,
        }).unwrap().requests.into_iter().next().unwrap();
        let digest = app.active_run(&fixture.session).unwrap().unwrap().invocation_config_digest.unwrap();
        app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest,
            received_at_ms: now, response: HostResponseEnvelope {
                request_id: original.request_id.clone(), attempt_id: original.attempt_id.clone(),
                delegated_invocation_id: original.delegated_invocation_id.clone(), effect_id: original.effect_id.clone(),
                host_id: original.host_id.clone(), lease_id: original.lease_id.clone(), session_id: original.session_id.clone(),
                turn_id: original.turn_id.clone(), catalog_revision: original.catalog_revision,
                availability_epoch: original.availability_epoch, delivery: original.delivery.clone(),
                sequence: original.resume_after_sequence.map_or(0, |n| n + 1), payload: HostResponse::ExecutionStarted,
            },
        }).unwrap();
        // A real expired lease, not a rewritten Store or forged death proof.
        while nola_runtime_driver::monotonic_epoch_millis() <= now + 100 {
            std::thread::sleep(Duration::from_millis(5));
        }
        let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
        let mut host = host.lock().unwrap();
        recover_scheduled_run_deliveries(&mut host, &config,
            Some(&nola_protocol::RuntimeEffectWorkReason::RecoverExpiredLease)).unwrap();
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        host.prepare_pending_effect_recovery(&mut emitter).unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let worker = crate::process_observation::current_worker_identity().unwrap().lease_identity().unwrap();
        let successor = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 1,
        }).unwrap().requests.into_iter().next().expect("expired work must no longer loop on an empty claim");
        assert_eq!(successor.effect_id, original.effect_id);
        assert_ne!(successor.request_id, original.request_id);
        assert_ne!(successor.delivery.as_ref().unwrap().attempt_id, original.delivery.as_ref().unwrap().attempt_id);
        host.commit_core_submission("cancel-rebound-target", AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "cancel-rebound-target".into(), reason: Some("fixture".into()),
        }, &mut emitter).unwrap();
        drop(host);
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let controls = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 64,
        }).unwrap();
        let control = controls.requests.iter().find(|request| matches!(&request.payload,
            nola_core::agent::host::HostRequest::Cancel { target_effect_id, .. } if target_effect_id == &original.effect_id))
            .expect("Core must issue the exact target Cancel");
        let error = validate_local_cancellation_stop(&fixture.session, &run, control,
            &original.request_id, original.delivery.as_ref()).unwrap_err();
        assert!(error.contains("current target delivery fence"), "{error}");
        // The latest request is not sufficient while its earlier started
        // attempt lacks a settled physical outcome.
        let error = validate_local_cancellation_stop(&fixture.session, &run, control,
            &successor.request_id, successor.delivery.as_ref()).unwrap_err();
        assert!(error.contains("current target delivery fence"), "{error}");
        for field in ["generation", "owner", "token"] {
            let mut changed = successor.delivery.clone().unwrap();
            match field {
                "generation" => changed.worker_generation.push_str("-foreign"),
                "owner" => changed.lease_owner.push_str("-foreign"),
                _ => changed.lease_token.push_str("-foreign"),
            }
            assert!(validate_local_cancellation_stop(&fixture.session, &run, control,
                &successor.request_id, Some(&changed)).is_err(), "{field}");
        }
        assert!(!controls.requests.iter().any(|request| request.effect_id == original.effect_id),
            "a cancelling target must never be dispatched beside its Cancel");
        let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
        let error = host.lock().unwrap().commit_core_submission("forbidden-cancelling-rebind",
            AgentSubmission::ResumeHostEffect { effect_id: original.effect_id.0.clone().into() }, &mut emitter).unwrap_err();
        assert!(error.contains("cannot be rebound"), "{error}");
    }

    #[test]
    fn pending_cancellation_is_not_rebound_during_warm_or_cold_host_recovery() { run_full_host_recovery_fixture(pending_cancellation_is_not_rebound_during_warm_or_cold_host_recovery_inner); }
    fn pending_cancellation_is_not_rebound_during_warm_or_cold_host_recovery_inner() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        let (_, run_id) = stage_registered_core_agent_turn_without_dispatch(&config, "fixture pending request").unwrap();
        let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        host.lock().unwrap().commit_core_submission("cancel-before-recovery", AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "pending-cancel".into(), reason: Some("fixture".into()),
        }, &mut emitter).unwrap();
        let mut application = crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let before = application.load_committed_host_requests(&run_id).unwrap();
        assert!(before.requests.iter().any(|request| matches!(request.payload, nola_core::agent::host::HostRequest::Cancel { .. })));
        for cold in [false, true] {
            if cold { invalidate_core_agent_session(&fixture.session).unwrap(); }
            let host = acquire_core_agent_host(&config, Vec::new()).unwrap();
            let mut host = host.lock().unwrap();
            // Exercise restored recoverable requests even in the warm case.
            host.resume_pending_once = true;
            assert!(host.runtime.execution.pending_effects.iter().any(|effect| !effect.recovery_rebind_allowed));
            host.prepare_pending_effect_recovery(&mut emitter).unwrap();
            let after = application.load_committed_host_requests(&run_id).unwrap();
            assert_eq!(serde_json::to_value(&before.requests).unwrap(), serde_json::to_value(&after.requests).unwrap());
        }
    }

    #[test]
    fn foreground_recovery_waits_for_original_host_before_validating_binding() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        let (_, run) = stage_registered_core_agent_turn_without_dispatch(&config, "retained Host fixture").unwrap();
        let host = active_core_agent_host(&fixture.session).unwrap().unwrap();
        let guard = host.lock().unwrap();
        std::thread::scope(|scope| {
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let (result_tx, result_rx) = std::sync::mpsc::channel();
            let owner = fixture.session.clone();
            let config = &config;
            let run = &run;
            scope.spawn(move || {
                let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
                let active = app.active_run(&owner).unwrap().unwrap();
                assert_eq!(&active.run_id, run);
                ready_tx.send(()).unwrap();
                result_tx.send(validate_recovery_host_binding(&mut app, &owner, &active, config)).unwrap();
            });
            ready_rx.recv().unwrap();
            let early = result_rx.recv_timeout(std::time::Duration::from_millis(100));
            drop(guard);
            let result = match early {
                Ok(result) => result,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => result_rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
                Err(error) => panic!("{error}"),
            };
            assert!(result.is_ok(), "Foreground recovery must wait for the original Host: {result:?}");
        });
    }

    #[test]
    fn cold_recovery_defers_busy_host_without_evicting_or_replacing_it() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        config.session.save_session = false;
        let host = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let slot = core_agent_host_slot(&fixture.session).unwrap();
        // Initialization can hold the slot before the Host is available.
        let slot_guard = slot.lock().unwrap();
        assert_eq!(retain_matching_or_forget_stale_core_agent_host(&fixture.session, "expected-run").unwrap(),
            RecoveryHostDisposition::Busy);
        drop(slot_guard);
        // Foreground execution can own the Host when the supervisor rechecks it.
        let host_guard = host.lock().unwrap();
        assert_eq!(retain_matching_or_forget_stale_core_agent_host(&fixture.session, "expected-run").unwrap(),
            RecoveryHostDisposition::Busy);
        assert_eq!(drain_registered_core_agent_run(&fixture.session, "expected-run", RuntimeExecutionControl::default(), None).unwrap(),
            RegisteredCoreAgentDrain::Busy);
        assert!(Arc::ptr_eq(&active_core_agent_host(&fixture.session).unwrap().unwrap(), &host));
        drop(host_guard);
        // Only after it becomes idle can the stale Run-less Host be removed.
        assert_eq!(retain_matching_or_forget_stale_core_agent_host(&fixture.session, "expected-run").unwrap(),
            RecoveryHostDisposition::Absent);
        assert!(active_core_agent_host(&fixture.session).unwrap().is_none());
    }

    #[test]
    fn cold_recovery_reports_poisoned_slot_as_error_not_busy() {
        let session = format!("poisoned-recovery-{}", uuid::Uuid::new_v4());
        let slot = core_agent_host_slot(&session).unwrap();
        let poisoned = Arc::clone(&slot);
        assert!(std::thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("fixture poison");
        }).join().is_err());
        assert!(retain_matching_or_forget_stale_core_agent_host(&session, "run").unwrap_err().contains("poisoned"));
        CORE_AGENT_HOSTS.get().unwrap().lock().unwrap().remove(&session);
    }

    #[test]
    fn host_cache_separates_retention_and_database_routes_for_the_same_session() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        config.session.save_session = false;
        let ephemeral = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let repeated = acquire_core_agent_host(&config, Vec::new()).unwrap();
        assert!(Arc::ptr_eq(&ephemeral, &repeated));

        config.session.save_session = true;
        let retained = acquire_core_agent_host(&config, Vec::new()).unwrap();
        assert!(!Arc::ptr_eq(&ephemeral, &retained));
        assert!(retained.lock().unwrap().execution_config.session.save_session);
        let repeated = acquire_core_agent_host(&config, Vec::new()).unwrap();
        assert!(Arc::ptr_eq(&retained, &repeated));

        std::env::set_var("NOLA_DB_PATH", fixture.root.join("other-route.sqlite"));
        let other_route = acquire_core_agent_host(&config, Vec::new()).unwrap();
        assert!(!Arc::ptr_eq(&retained, &other_route));
        let repeated = acquire_core_agent_host(&config, Vec::new()).unwrap();
        assert!(Arc::ptr_eq(&other_route, &repeated));

        config.session.save_session = false;
        let new_ephemeral = acquire_core_agent_host(&config, Vec::new()).unwrap();
        assert!(!Arc::ptr_eq(&other_route, &new_ephemeral));
        assert!(!Arc::ptr_eq(&ephemeral, &new_ephemeral));
        assert!(!new_ephemeral.lock().unwrap().execution_config.session.save_session);
    }

    #[test]
    fn guarded_goal_controls_preserve_the_exact_pending_run_in_warm_and_cold_hosts() {
        use nola_core::agent::goal::{GoalRecordStatus, GoalRequirement, GoalRuntimeCommand, GoalStatus};
        let _environment = crate::store::test_env_lock();
        for cold in [false, true] {
            let (fixture, mut config) = binding_config();
            config.session.session_id = Some(fixture.session.clone());
            config.session.save_session = true;
            config.tool_settings.skill_dirs = Some(Vec::new());
            config.tool_settings.tools_enabled = false;
            config.tool_settings.tool_allowlist = Some(Vec::new());
            config.runtime.output_admission =
                Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), false).unwrap());
            submit_agent_core_goal_command(&config, GoalRuntimeCommandEnvelope {
                operation_id: "goal-control:start".into(),
                command: GoalRuntimeCommand::Start {
                    id: "goal-control".into(), objective: "Finish the owned work".into(),
                    constraints: Vec::new(), requirements: vec![GoalRequirement {
                        acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                        semantic_revision: 1, id: "owned-work".into(), description: "The owned work is finished".into(),
                        dependencies: Vec::new(), status: GoalRecordStatus::Pending,
                        evidence_refs: Vec::new(),
                    }], now_epoch_ms: 1000,
                },
            }).unwrap();
            let _shared = acquire_core_agent_host(&config, Vec::new()).unwrap();
            // Starting a Goal already admits its canonical Run. The retained
            // Host must recover that Run, never manufacture another user turn.
            let mut application = crate::application_port::CliApplicationPort::persistent_default().unwrap();
            let run_id = application.active_run(&fixture.session).unwrap().unwrap().run_id;
            let pending = application.load_committed_host_requests(&run_id).unwrap();
            assert!(!pending.requests.is_empty(), "fixture must retain actual pending execution");
            if cold {
                invalidate_core_agent_session(&fixture.session).unwrap();
            }
            for (operation, command, _status) in [
                ("pause", GoalRuntimeCommand::Pause { status: GoalStatus::Paused,
                    wake_at_epoch_ms: None, now_epoch_ms: 1200 }, GoalStatus::Paused),
                ("resume", GoalRuntimeCommand::Resume { now_epoch_ms: 1300 }, GoalStatus::Running),
            ] {
                let current = crate::session::load_session_goal_projection(&fixture.session).unwrap().unwrap();
                let command = GoalRuntimeCommand::GuardedControl {
                    expected_goal_id: current.active.as_ref().unwrap().id.clone(),
                    expected_control_token: current.control_token().unwrap(), control: Box::new(command),
                };
                let result = submit_agent_core_goal_command(&config, GoalRuntimeCommandEnvelope {
                    operation_id: format!("goal-control:{operation}"), command,
                });
                if operation == "resume" {
                    assert!(result.unwrap_err().contains("pending exact Run stop confirmation"));
                } else { result.unwrap(); }
                assert_eq!(crate::session::load_session_goal_projection(&fixture.session)
                    .unwrap().unwrap().active.unwrap().status, GoalStatus::Paused);
                let active = application.active_run(&fixture.session).unwrap().unwrap();
                assert_eq!(active.run_id, run_id);
                let remaining = application.load_committed_host_requests(&run_id).unwrap();
                assert!(!remaining.requests.is_empty());
                for request in &remaining.requests {
                    match &request.payload {
                        nola_core::agent::host::HostRequest::Cancel { target_effect_id, .. } => {
                            assert!(pending.requests.iter().any(|original| original.effect_id == *target_effect_id),
                                "cancellation must target the original owned request");
                        }
                        _ => panic!("pause/resume must not dispatch another model request before cancellation reconciles"),
                    }
                }
            }
        }
    }

    #[test]
    fn memory_admission_is_bound_and_restore_does_not_upgrade_frozen_policy() {
        let _environment = crate::store::test_env_lock();
        let (_fixture, mut config) = binding_config();
        let mut disabled = config.runtime.admitted_memory_policy.clone().unwrap();
        disabled.long_term_enabled = false;
        disabled.automatic_extraction_enabled = false;
        restore_host_memory_admission(&mut config, &disabled).unwrap();
        assert_eq!(
            config.runtime.admitted_memory_policy.as_ref(),
            Some(&disabled)
        );
        let original = host_invocation_binding(&config).unwrap();
        config
            .runtime
            .admitted_memory_policy
            .as_mut()
            .unwrap()
            .search_limit += 1;
        let changed = host_invocation_binding(&config).unwrap();
        assert_ne!(
            original.execution_policy_sha256,
            changed.execution_policy_sha256
        );
        assert!(original.require_exact(&changed).is_err());
        config.runtime.admitted_memory_policy = None;
        assert!(host_invocation_binding(&config)
            .unwrap_err()
            .contains("memory policy has not been admitted"));
    }

    #[test]
    fn cold_host_accepts_distinct_core_session_and_fences_checkpoint_advance() {
        cold_host_preserves_origin_and_checkpoint(nola_host_protocol::HostMacInputOrigin::InteractiveUser);
    }

    #[test]
    fn scheduled_cold_host_preserves_original_origin_and_rejects_changed_binding() {
        cold_host_preserves_origin_and_checkpoint(nola_host_protocol::HostMacInputOrigin::ScheduledAction);
    }

    fn cold_host_preserves_origin_and_checkpoint(origin: nola_host_protocol::HostMacInputOrigin) {
        use nola_core::agent::AgentInputOrigin;
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        config.runtime.mac_launch = Some(mac_launch_fixture(&config));
        config.runtime.mac_launch.as_mut().unwrap().input_origin = origin;
        config.input.host_input_origin = match origin {
            nola_host_protocol::HostMacInputOrigin::InteractiveUser => AgentInputOrigin::InteractiveUser,
            nola_host_protocol::HostMacInputOrigin::ScheduledAction => AgentInputOrigin::ScheduledAction,
        };
        config.runtime.output_admission =
            Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), false).unwrap());
        let original_host = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let mut original_host = original_host.lock().unwrap();
        original_host.session_store.as_ref().unwrap().apply_initial_creation_boundary(&mut config.session).unwrap();
        let mut core_config = original_host.runtime.core_config().unwrap().clone();
        let core_session = format!("actual-core-{}", uuid::Uuid::new_v4());
        core_config.session_id = Some(SessionId::from_string(core_session.clone()));
        let input = host_user_input_items(&config, "Recover the owned Run");
        let submission = AgentSubmission::UserInput {
            items: input.clone(),
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("distinct-core-run".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        };
        let begin =
            canonical_begin_turn_input(&config, &fixture.session, &input, core_config, submission)
                .unwrap();
        let accepted = original_host
            .session_store
            .as_mut()
            .unwrap()
            .begin_core_turn(begin)
            .unwrap();
        assert_eq!(accepted.execution.control.session_id.as_str(), core_session);
        let run_id = accepted.begin.run_id.clone();
        drop(original_host);
        invalidate_core_agent_session(&fixture.session).unwrap();

        // Resume has no new input channel. The absent-input parser default is
        // not allowed to replace the original provenance in the frozen binding.
        config.input.host_input_origin = AgentInputOrigin::InteractiveUser;
        let mut application =
            crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let active = application.active_run(&fixture.session).unwrap().unwrap();
        let binding =
            validate_recovery_host_binding(&mut application, &fixture.session, &active, &config)
                .unwrap();
        assert_eq!(binding, Some(host_invocation_binding(&config).unwrap()));
        let (_, exact) =
            load_exact_recovery_checkpoint(&mut application, &fixture.session, &active).unwrap();
        assert_eq!(
            exact.core_session_id.as_deref(),
            Some(core_session.as_str())
        );
        assert_eq!(exact.conversation_id, fixture.session);
        assert_eq!(exact.host_invocation.mac_launch, config.runtime.mac_launch);
        let before_rejected_resume = application.load_checkpoint(&fixture.session).unwrap().unwrap();
        let mut changed_origin = config.clone();
        changed_origin.runtime.mac_launch.as_mut().unwrap().input_origin = match origin {
            nola_host_protocol::HostMacInputOrigin::InteractiveUser => nola_host_protocol::HostMacInputOrigin::ScheduledAction,
            nola_host_protocol::HostMacInputOrigin::ScheduledAction => nola_host_protocol::HostMacInputOrigin::InteractiveUser,
        };
        assert!(validate_recovery_host_binding(&mut application, &fixture.session, &active, &changed_origin).is_err());
        let mut changed_launch = config.clone();
        changed_launch.runtime.mac_launch.as_mut().unwrap().project_path = "/foreign-workspace".into();
        assert!(validate_recovery_host_binding(&mut application, &fixture.session, &active, &changed_launch).is_err());
        assert_eq!(serde_json::to_value(application.load_checkpoint(&fixture.session).unwrap().unwrap()).unwrap(),
            serde_json::to_value(before_rejected_resume).unwrap());
        application
            .reduce_and_commit_active_submission(
                nola_application::ReduceAndCommitActiveSubmission {
                    conversation_id: fixture.session.clone(),
                    run_id: run_id.clone(),
                    invocation_config_digest: accepted.invocation_config_digest,
                    transition_id: "advance-owned-checkpoint".into(),
                    submission: AgentSubmission::Interrupt {
                        goal_control: None, cancellation_id: "fixture-checkpoint-advance".into(),
                        reason: Some("fixture checkpoint advance".into()),
                    },
                    committed_at_ms: nola_runtime_driver::monotonic_epoch_millis(),
                },
            )
            .unwrap();
        let advanced = application
            .load_checkpoint(&fixture.session)
            .unwrap()
            .unwrap();
        assert!(validate_exact_recovery_checkpoint(&exact, &advanced)
            .unwrap_err()
            .contains("changed after admission"));
        let active = application.active_run(&fixture.session).unwrap().unwrap();
        let (fresh, _) =
            load_exact_recovery_checkpoint(&mut application, &fixture.session, &active).unwrap();
        assert_eq!(fresh.session_id, core_session);

        let cold_host = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let cold_host = cold_host.lock().unwrap();
        assert_eq!(
            cold_host.session_id, fixture.session,
            "registry and Store route remain canonical"
        );
        assert_eq!(cold_host.runtime.control_ref().session_id.as_str(), core_session);
        assert_eq!(
            cold_host.runtime.config.session_id.as_ref().unwrap().as_str(),
            core_session
        );
        assert_eq!(
            cold_host
                .session_store
                .as_ref()
                .unwrap()
                .active_core_run_id(),
            Some(run_id.as_str())
        );
    }

    #[test]
    fn memory_admission_rejects_revoked_authority_without_upgrading_disabled_run() {
        let mut enabled = nola_core::memory::MemoryPolicy::default();
        enabled.long_term_enabled = true;
        enabled.automatic_extraction_enabled = true;
        let mut no_extraction = enabled.clone();
        no_extraction.automatic_extraction_enabled = false;
        let mut disabled = no_extraction.clone();
        disabled.long_term_enabled = false;
        assert!(validate_memory_policy_ceiling(&enabled, &no_extraction).is_err());
        assert!(validate_memory_policy_ceiling(&no_extraction, &disabled).is_err());
        assert!(validate_memory_policy_ceiling(&disabled, &enabled).is_ok());
        assert!(validate_memory_policy_ceiling(&enabled, &enabled).is_ok());
    }

    #[test]
    fn queued_input_executes_in_its_own_run_hot_and_after_cold_resume() {
        let _environment = crate::store::test_env_lock();
        for cold in [false, true] {
            let (fixture, mut config) = binding_config();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            config.model_settings.endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
            config.session.session_id = Some(fixture.session.clone());
            config.session.save_session = true;
            config.tool_settings.skill_dirs = Some(Vec::new());
            config.tool_settings.tools_enabled = false;
            config.tool_settings.tool_allowlist = Some(Vec::new());
            config.runtime.memory_enabled = Some(false);
            config.runtime.admitted_memory_policy = None;
            freeze_host_memory_admission(&mut config).unwrap();
            config.runtime.output_admission = Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), false).unwrap());
            let stop = fixture.stop.clone();
            let posts = Arc::new(AtomicUsize::new(0));
            let observed = posts.clone();
            let server = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                    let (mut stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        Err(error) => panic!("fixture accept: {error}"),
                    };
                    // accept() readiness does not mean the client has written
                    // its request yet. Bound reads by this fixture's overall
                    // deadline, not a separate short startup race; explicitly
                    // clear inherited nonblocking state on accepted sockets.
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())
                        .max(Duration::from_millis(1)))).unwrap();
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let path = line.split_whitespace().nth(1).unwrap().to_owned();
                    let mut length = 0usize;
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header == "\r\n" { break; }
                        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse().unwrap();
                        }
                    }
                    assert!(length < 2_000_000);
                    let mut request = vec![0; length];
                    reader.read_exact(&mut request).unwrap();
                    let (mime, body) = if path == "/v1/models" {
                        ("application/json", json!({"data":[{"id":"fixture-model","context_length":32768}]}).to_string())
                    } else {
                        assert_eq!(path, "/v1/chat/completions");
                        let ordinal = observed.fetch_add(1, Ordering::SeqCst);
                        assert!(ordinal < 2, "queued input must not repeat a physical request");
                        let payload: Value = serde_json::from_slice(&request).unwrap();
                        assert_eq!(payload["messages"].to_string().contains("Second question"), ordinal == 1,
                            "queued input belongs only to the successor request");
                        let delta = json!({"choices":[{"index":0,"delta":{"content":format!("Answer {}.",ordinal+1)},"finish_reason":null}]});
                        let end = json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                        ("text/event-stream", format!("data: {delta}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
                    };
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
            });
            let result = (|| -> Result<(), String> {
                let host = acquire_core_agent_host(&config, Vec::new())?;
                {
                    let mut host = host.lock().map_err(|_| "fixture host poisoned")?;
                    host.session_store.as_ref().unwrap().apply_initial_creation_boundary(&mut config.session)?;
                    let input = host_user_input_items(&config, "First question");
                    let submission = AgentSubmission::UserInput {
                        items: input.clone(), turn_context_items: vec![], turn_context_platform: Default::default(),
                        turn_context_components: vec![], submission_id: Some("first-run".into()),
                        submitted_at_epoch_ms: Some(1000), delivery: AgentInputDelivery::CurrentTurn,
                    };
                    let mut admitted_config = host.runtime.core_config()?.clone();
                    admitted_config.turn_identity.host_invocation = Some(host_invocation_binding(&config)?);
                    host.runtime.driver_mut().set_execution_settings(config.host_execution_settings());
                    let begin = canonical_begin_turn_input(&config, &fixture.session, &input,
                        admitted_config, submission)?;
                    let accepted = host.session_store.as_mut().unwrap().begin_core_turn(begin)
                        .map_err(|error| error.to_string())?;
                    host.runtime.adopt_execution_state(accepted.execution).map_err(format_runtime_error)?;
                    host.session_store.as_mut().unwrap().reduce_active_core_submission("queue-next".into(),
                        AgentSubmission::UserInput {
                            items: vec![AgentItem::user_message(ItemId::from_string("queued-user".into()), "Second question")],
                            turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
                            submission_id: Some("queued-run".into()), submitted_at_epoch_ms: Some(1001),
                            delivery: AgentInputDelivery::NextTurn,
                        })?;
                    let CoreAgentHost { runtime, session_store, .. } = &mut *host;
                    let control = runtime.execute_committed_scope(session_store.as_mut().unwrap(), &mut |_| {}, !cold, &mut || host_invocation_binding(&config))
                        .map_err(format_runtime_error)?;
                    assert_eq!(control.turn_status, AgentTurnStatus::Completed);
                    assert_eq!(runtime.terminal_run_id()?, if cold { "first-run" } else { "queued-run" },
                        "terminal ownership follows the actually drained Run");
                }
                if cold {
                    assert_eq!(posts.load(Ordering::SeqCst),1,"exact-Run drain must not execute its successor");
                    drop(host);
                    invalidate_core_agent_session(&fixture.session)?;
                    let captured = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
                    let sink = captured.clone();
                    let mut emitter = Emitter::with_event_sink(move |event| sink.lock().unwrap().push(event.clone()));
                    assert!(resume_agent_core_checked(&config,&mut emitter,RuntimeExecutionControl::default(),Some("first-run")).is_err());
                    let cancelled = RuntimeExecutionControl::default();
                    cancelled.request();
                    assert!(resume_agent_core_checked(&config,&mut emitter,cancelled,None)?.is_none());
                    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
                    assert!(application.active_run(&fixture.session)?.is_none(), "cancelled resume must not admit a successor");
                    assert_eq!(posts.load(Ordering::SeqCst), 1, "cancelled caller must not consume pending successor");
                    let resumed = resume_agent_core_checked(&config,&mut emitter,RuntimeExecutionControl::default(),None)?;
                    assert!(resumed.is_some());
                    let terminal_runs = captured.lock().unwrap().iter()
                        .filter(|event| event["type"] == "assistant_run_terminal_committed_v1")
                        .map(|event| event["run_id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
                    assert_eq!(terminal_runs, vec!["queued-run"], "publish the exact resumed successor seal");
                }
                let connection = rusqlite::Connection::open_with_flags(fixture.root.join("never-opened.sqlite"),
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|error| error.to_string())?;
                let rows = connection.prepare("SELECT id,status FROM runs WHERE kind='conversation' ORDER BY created_at_ms")
                    .map_err(|error| error.to_string())?.query_map([],|row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))
                    .map_err(|error| error.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?;
                assert_eq!(rows,vec![("first-run".into(),"sealed".into()),("queued-run".into(),"sealed".into())]);
                assert_eq!(posts.load(Ordering::SeqCst),2);
                Ok(())
            })();
            fixture.stop.store(true,Ordering::SeqCst);
            server.join().unwrap();
            result.unwrap();
        }
    }

    #[test]
    fn memory_revocation_before_model_dispatch_commits_rejection_and_settles_run() {
        let _environment = crate::store::test_env_lock();
        let (mut fixture, mut config) = binding_config();
        fixture.environment.push((
            "NOLA_LONG_TERM_MEMORY_ENABLED",
            std::env::var_os("NOLA_LONG_TERM_MEMORY_ENABLED"),
        ));
        std::env::remove_var("NOLA_LONG_TERM_MEMORY_ENABLED");
        let provider = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        provider.set_nonblocking(true).unwrap();
        config.model_settings.endpoint = format!("http://{}/v1", provider.local_addr().unwrap());
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = false;
        config.tool_settings.tool_allowlist = Some(Vec::new());
        config.runtime.memory_enabled = Some(true);
        config.runtime.admitted_memory_policy = None;
        freeze_host_memory_admission(&mut config).unwrap();
        config.runtime.output_admission =
            Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), false).unwrap());
        let host = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let mut host = host.lock().unwrap();
        host.session_store.as_ref().unwrap().apply_initial_creation_boundary(&mut config.session).unwrap();
        let input = host_user_input_items(&config, "Answer briefly");
        let submission = AgentSubmission::UserInput {
            items: input.clone(),
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("revoked-memory-run".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        };
        let begin = canonical_begin_turn_input(
            &config,
            &fixture.session,
            &input,
            host.runtime.core_config().unwrap().clone(),
            submission,
        )
        .unwrap();
        let accepted = host
            .session_store
            .as_mut()
            .unwrap()
            .begin_core_turn(begin)
            .unwrap();
        let run_id = accepted.begin.run_id.clone();
        assert!(
            accepted
                .execution
                .configuration
                .memory_policy
                .long_term_enabled
        );
        host.runtime
            .adopt_execution_state(accepted.execution)
            .unwrap();
        // Revoke only after the real Application Begin transaction committed.
        std::env::set_var("NOLA_LONG_TERM_MEMORY_ENABLED", "false");
        let CoreAgentHost {
            runtime,
            session_store,
            ..
        } = &mut *host;
        let snapshot = runtime
            .execute_committed_application_port(session_store.as_mut().unwrap(), &mut |_| {})
            .unwrap();
        assert!(snapshot.turn_status.is_terminal());
        assert_eq!(runtime.execution.pending_effects.len(), 0);
        assert_eq!(
            provider.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "revocation must precede any provider connection"
        );
        let mut application =
            crate::application_port::CliApplicationPort::persistent_default().unwrap();
        assert!(application.active_run(&fixture.session).unwrap().is_none());
        let connection =
            rusqlite::Connection::open(fixture.root.join("never-opened.sqlite")).unwrap();
        let payloads = connection
            .prepare("SELECT payload_json FROM host_event_inbox WHERE run_id=?1")
            .unwrap()
            .query_map([&run_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(payloads
            .iter()
            .any(|payload| payload.contains("execution_rejected")
                && payload.contains("memory_authority_unavailable")));
        assert!(!payloads
            .iter()
            .any(|payload| payload.contains("execution_started")));
        let pending: i64 = connection
            .query_row(
                "SELECT count(*) FROM effect_outbox WHERE run_id=?1 AND status='pending'",
                [&run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
    }

    #[test]
    fn rejected_first_admission_can_retry_without_inventing_a_checkpoint() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = false;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.runtime.output_admission = Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), false).unwrap());
        let shared = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let mut host = shared.lock().unwrap();
        // This direct Begin fixture bypasses the outer host entrypoint. Carry
        // its exact Create receipt, not an invented or refreshed CAS version.
        host.session_store.as_ref().unwrap()
            .apply_initial_creation_boundary(&mut config.session).unwrap();
        let input = host_user_input_items(&config, "Retry a rejected admission");
        let submission = AgentSubmission::UserInput {
            items: input.clone(), turn_context_items: Vec::new(),
            turn_context_platform: Default::default(), turn_context_components: Vec::new(),
            submission_id: Some("retry-admission".into()), submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        };
        let valid = canonical_begin_turn_input(&config, &fixture.session, &input,
            host.runtime.core_config().unwrap().clone(), submission).unwrap();
        let idle_rejected = host.session_store.as_mut().unwrap().reduce_idle_core_submission(
            valid.core_config.clone(), valid.submission.clone());
        assert!(matches!(&idle_rejected,
            Err(nola_service_client::ApplicationServiceCommandError::Rejected { .. })));
        host.runtime.requires_committed_restore = true;
        assert!(host.runtime.receive_admission(idle_rejected).is_err());
        assert!(!host.runtime.requires_committed_restore,
            "a rejected preflight must not become an unknown execution outcome");
        let mut invalid = valid.clone();
        if let AgentSubmission::UserInput { submission_id, .. } = &mut invalid.submission {
            *submission_id = None;
        }
        host.runtime.requires_committed_restore = true;
        let rejected = host.session_store.as_mut().unwrap().begin_core_turn(invalid);
        assert!(matches!(&rejected, Err(nola_service_client::ApplicationServiceCommandError::Rejected { .. })));
        assert!(host.runtime.receive_admission(rejected).is_err());
        assert!(!host.runtime.requires_committed_restore);
        assert!(!host.session_store.as_ref().unwrap().has_active_core_turn());
        host.recover_projection_if_needed().unwrap();
        let accepted = host.session_store.as_mut().unwrap().begin_core_turn(valid);
        let accepted = host.runtime.receive_admission(accepted).unwrap();
        assert!(host.runtime.requires_committed_restore, "committed reply is not yet adopted");
        host.runtime.adopt_execution_state(accepted.execution).unwrap();
        assert!(!host.runtime.requires_committed_restore);
        assert!(host.session_store.as_ref().unwrap().has_active_core_turn());
        let error = nola_service_client::ApplicationServiceCommandError::OutcomeUnknown {
            client_id: "host".into(), request_id: "uncertain-admission".into(),
            code: "committed_result_decode_failed".into(), message: "invalid committed reply".into(),
            reopen_required: true,
        };
        assert!(host.runtime.receive_admission::<nola_application::AcceptedIdleSubmission>(Err(error)).is_err());
        assert!(host.runtime.require_adopted_state().is_err());
    }

    #[test]
    fn committed_checkpoint_restore_is_atomic_and_accepts_legal_configuration_changes() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = false;
        config.tool_settings.skill_dirs = Some(Vec::new());
        // Admission without metadata probing: no model/provider request is needed.
        config.runtime.output_admission =
            Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), false).unwrap());
        let host = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let mut host = host.lock().unwrap();
        let before_config = serde_json::to_value(&host.runtime.config).unwrap();
        let before_execution = serde_json::to_value(&host.runtime.execution).unwrap();
        let before_snapshot = serde_json::to_value(host.runtime.control()).unwrap();
        let before_catalog = serde_json::to_value(host.runtime.driver_mut().catalog()).unwrap();
        let baseline = AgentCore::try_new(host.runtime.config.clone()).unwrap()
            .checkpoint(CheckpointReason::Manual);
        let mut rejected = baseline.clone();
        rejected.state.session_id = SessionId::from_string("foreign-session".into());
        rejected.state.configuration.memory_policy.long_term_enabled =
            !rejected.state.configuration.memory_policy.long_term_enabled;
        rejected.state.host.catalog.revision += 1;
        assert!(AgentCore::restore(host.runtime.config.clone(), rejected).is_err());
        let mut rejected_execution = host.runtime.execution.clone();
        rejected_execution.configuration.session_id = Some(SessionId::from_string("foreign-session".into()));
        rejected_execution.configuration.memory_policy.long_term_enabled =
            !rejected_execution.configuration.memory_policy.long_term_enabled;
        assert!(host.runtime.adopt_execution_state(rejected_execution).is_err());
        assert!(host.runtime.requires_committed_restore);
        assert!(host.runtime.core_config().is_err(), "stale config must not seed another submission");
        assert!(host.runtime.require_adopted_state().is_err());
        {
            let CoreAgentHost { runtime, session_store, .. } = &mut *host;
            let error = runtime.execute_committed_application_port(
                session_store.as_mut().unwrap(),
                &mut |_| panic!("a fenced runtime must not dispatch or publish"),
            ).unwrap_err();
            assert!(format_runtime_error(error).contains("requires recovery"));
            assert!(runtime.requires_committed_restore);
        }
        assert_eq!(
            serde_json::to_value(&host.runtime.config).unwrap(),
            before_config
        );
        assert_eq!(
            serde_json::to_value(&host.runtime.execution).unwrap(),
            before_execution
        );
        assert_eq!(
            serde_json::to_value(host.runtime.control()).unwrap(),
            before_snapshot
        );
        assert_eq!(
            serde_json::to_value(host.runtime.driver_mut().catalog()).unwrap(),
            before_catalog
        );

        // Produce the accepted checkpoint through actual Core submissions,
        // rather than forging a successful restore's configuration fields.
        let mut core =
            AgentCore::restore(host.runtime.config.clone(), baseline)
                .unwrap();
        let mut memory = host.runtime.config.memory_policy.clone();
        memory.long_term_enabled = !memory.long_term_enabled;
        memory.automatic_extraction_enabled = false;
        core.submit(AgentSubmission::UpdateSessionConfiguration {
            patch: nola_core::agent::AgentSessionConfigurationPatch {
                memory_policy: Some(memory.clone()),
                ..Default::default()
            },
        })
        .unwrap();
        let mut catalog = host.runtime.driver_mut().catalog().clone();
        catalog.revision += 1;
        core.submit(AgentSubmission::UpdateHostCatalog {
            catalog: catalog.clone(),
        })
        .unwrap();
        let expected_snapshot = serde_json::to_value(core.control_snapshot()).unwrap();
        let expected_execution = serde_json::to_value(core.host_execution_state()).unwrap();
        host.runtime
            .adopt_execution_state(core.host_execution_state())
            .unwrap();
        assert!(!host.runtime.requires_committed_restore);
        assert!(host.runtime.require_adopted_state().is_ok());
        assert_eq!(host.runtime.config.memory_policy, memory);
        assert_eq!(
            serde_json::to_value(&host.runtime.execution).unwrap(),
            expected_execution
        );
        assert_eq!(
            serde_json::to_value(host.runtime.control()).unwrap(),
            expected_snapshot
        );
        assert_eq!(
            host.runtime.driver_mut().catalog().revision,
            catalog.revision
        );
        assert_eq!(
            host.runtime.config.host_catalog.as_ref().unwrap().revision,
            catalog.revision
        );
    }

    #[test]
    fn cached_host_recovers_committed_state_from_its_original_ephemeral_store() {
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = false;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.runtime.output_admission =
            Some(crate::host_output_admission::admit(&config.model_invocation_settings().unwrap(), false).unwrap());
        let shared = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let expected;
        let committed_memory;
        {
            let mut host = shared.lock().unwrap();
            let core_config = host.runtime.core_config().unwrap().clone();
            let mut memory = core_config.memory_policy.clone();
            memory.long_term_enabled = !memory.long_term_enabled;
            memory.automatic_extraction_enabled = false;
            committed_memory = memory.clone();
            // Commit through the real Application port, but do not adopt its
            // result. This creates an actual Store-ahead-of-runtime boundary.
            let accepted = host.session_store.as_mut().unwrap()
                .reduce_idle_core_submission(core_config, AgentSubmission::UpdateSessionConfiguration {
                    patch: nola_core::agent::AgentSessionConfigurationPatch {
                        memory_policy: Some(memory), ..Default::default()
                    },
                }).unwrap();
            expected = serde_json::to_value(host.session_store.as_mut().unwrap().load_host_execution_state().unwrap()).unwrap();
            let mut invalid = accepted.execution;
            invalid.configuration.session_id = Some(SessionId::from_string("foreign-session".into()));
            assert!(host.runtime.adopt_execution_state(invalid).is_err());
            assert_ne!(host.runtime.config.memory_policy, committed_memory);
            let original_store = host.session_store.take();
            assert!(host.recover_projection_if_needed().is_err());
            assert!(host.runtime.requires_committed_restore,
                "a missing original Store must not clear the execution fence");
            host.session_store = original_store;
        }
        let recovered = acquire_core_agent_host(&config, Vec::new()).unwrap();
        assert!(Arc::ptr_eq(&shared, &recovered), "recovery must not replace the retained ephemeral Host");
        let host = recovered.lock().unwrap();
        assert!(!host.runtime.requires_committed_restore);
        assert_eq!(host.runtime.core_config().unwrap().memory_policy, committed_memory);
        assert_eq!(serde_json::to_value(&host.runtime.execution).unwrap(), expected);
    }

    #[test]
    fn resume_wrong_run_cannot_recover_other_runs_observed_orphan_delivery() {
        observed_recovery_fixture(false, false);
    }

    #[test]
    fn observed_recovery_atomically_releases_dead_session_and_delivery_ownership() {
        observed_recovery_fixture(true, false);
    }

    #[test]
    fn observed_recovery_releases_dead_session_before_first_delivery_without_settling_run() {
        observed_recovery_fixture(true, true);
    }

    fn observed_recovery_fixture(with_session_lease: bool, before_delivery: bool) {
        // This finite debug integration path nests Host/Application recovery,
        // like permission_entrypoint_fixture below; keep its test budget local.
        std::thread::Builder::new()
            .name("observed-recovery-fixture".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || observed_recovery_fixture_inner(with_session_lease, before_delivery))
            .expect("spawn observed recovery fixture")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    }

    fn observed_recovery_fixture_inner(with_session_lease: bool, before_delivery: bool) {
        use nola_application::{
            ApplicationCommand, ClaimCommittedHostRequests, ReduceAndAcceptHostResponse,
        };
        use nola_core::agent::host::*;
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        let binding = host_invocation_binding(&config).unwrap();
        let mut application =
            crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let capability = HostCapabilityDescriptor {
            id: "model.stream".into(),
            title: "Fixture model".into(),
            route: HostCapabilityRoute::Model,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Internal,
            permission_domain: None,
            input_schema: json!({"type":"object"}),
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
        let begun = application
            .begin_assistant_turn(BeginAssistantTurn {
                expected_configuration_revision: None, expected_parent_message_id: None,
                conversation_id: fixture.session.clone(),
                conversation_title: "Wrong Run recovery".into(),
                user_message_id: "fixture-user".into(),
                user_part_id: "fixture-user:text".into(),
                user_text: "Keep this Run owned".into(),
                user_content_json: None,
                assistant_message_id: "current-run:assistant".into(),
                context_checkpoint_id: None,
                core_config: AgentCoreConfig {
                    turn_identity: AgentTurnIdentity {
                        host_invocation: Some(binding.clone()),
                        ..crate::test_model_identity::identity()
                    },
                    session_id: Some(SessionId::from_string(fixture.session.clone())),
                    model: Some("fixture-model".into()),
                    host_catalog: Some(HostCapabilityCatalog {
                        host_id: HostId("fixture-host".into()),
                        revision: 1,
                        availability_epoch: 1,
                        lease: None,
                        capabilities: [("model.stream".into(), capability)].into_iter().collect(),
                    }),
                    ..Default::default()
                },
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("fixture-user".into()),
                        "Keep this Run owned",
                    )],
                    turn_context_items: vec![],
                    turn_context_platform: Default::default(),
                    turn_context_components: vec![],
                    submission_id: Some("current-run".into()),
                    submitted_at_epoch_ms: Some(1000),
                    delivery: AgentInputDelivery::CurrentTurn,
                },
                created_at_ms: 1000,
            })
            .unwrap();
        // Observe an actual isolated child exit, not a guessed OS PID. No
        // provider or user's process is launched or signalled by this fixture.
        let mut child = crate::process_observation::test_exited_child();
        let dead_pid = child.id();
        assert!(child.wait().unwrap().success());
        assert!(!crate::process_observation::process_id_is_alive(dead_pid));
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let session_lease = with_session_lease.then(|| {
            nola_store_port::workspace_ownership::StoredSessionRunLease {
                session_id: fixture.session.clone(),
                owner: "isolated-dead-owner".into(),
                pid: dead_pid,
                process_start: "isolated-child-start".into(),
                acquired_at_ms: now,
            }
        });
        if let Some(lease) = &session_lease {
            let acquired: bool = application
                .execute(ApplicationCommand::AcquireSessionRunLease {
                    input: serde_json::from_value(serde_json::to_value(lease).unwrap()).unwrap(),
                })
                .unwrap();
            assert!(acquired);
        }
        if before_delivery {
            assert_eq!(
                recover_observed_orphaned_deliveries_for_run(
                    &mut application,
                    &fixture.session,
                    &begun.begin.run_id,
                    &binding
                )
                .unwrap(),
                None
            );
            let remaining: Option<nola_application::SessionRunLease> = application
                .execute(ApplicationCommand::GetSessionRunLease {
                    session_id: fixture.session.clone(),
                })
                .unwrap();
            assert!(remaining.is_none());
            assert_eq!(
                application
                    .active_run(&fixture.session)
                    .unwrap()
                    .unwrap()
                    .run_id,
                begun.begin.run_id
            );
            return;
        }
        let identity = crate::process_observation::test_worker_lease(dead_pid);
        let request = application
            .claim_committed_host_requests(ClaimCommittedHostRequests {
                run_id: begun.begin.run_id.clone(),
                worker_generation: identity.worker_generation.clone(),
                lease_owner: identity.lease_owner.clone(),
                claimed_at_ms: now,
                lease_expires_at_ms: now + 60_000,
                limit: 1,
            })
            .unwrap()
            .requests
            .into_iter()
            .next()
            .unwrap();
        let lease = request.delivery.clone().unwrap();
        application
            .reduce_and_accept_host_response(ReduceAndAcceptHostResponse {
                conversation_id: fixture.session.clone(),
                run_id: begun.begin.run_id.clone(),
                invocation_config_digest: begun.invocation_config_digest,
                response: HostResponseEnvelope {
                    request_id: request.request_id,
                    attempt_id: request.attempt_id,
                    delegated_invocation_id: request.delegated_invocation_id,
                    effect_id: request.effect_id,
                    host_id: request.host_id,
                    lease_id: request.lease_id,
                    session_id: request.session_id,
                    turn_id: request.turn_id,
                    catalog_revision: request.catalog_revision,
                    availability_epoch: request.availability_epoch,
                    delivery: request.delivery,
                    sequence: 1,
                    payload: HostResponse::ExecutionStarted,
                },
                received_at_ms: now,
            })
            .unwrap();
        let database = fixture.root.join("never-opened.sqlite");
        let before = nola_store::NolaStore::open(&database)
            .unwrap()
            .effect_delivery_attempt(&lease.attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            before.state,
            nola_domain::EffectDeliveryAttemptState::ExecutionStarted
        );
        let mut emitter = Emitter::with_event_sink(|_| {});
        let error = resume_agent_core_expected_with_cancellation(
            &config,
            &mut emitter,
            RuntimeExecutionControl::default(),
            "wrong-run",
        )
        .unwrap_err();
        assert!(error.contains("expected Run identity"), "{error}");
        let unchanged = || {
            nola_store::NolaStore::open(&database)
                .unwrap()
                .effect_delivery_attempt(&lease.attempt_id)
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            serde_json::to_value(unchanged()).unwrap(),
            serde_json::to_value(&before).unwrap(),
            "wrong-Run admission must not interrupt the current Run's physical delivery"
        );

        // The mutation itself is fenced too: even valid, freshly observed
        // current leases do not authorize recovery on behalf of another Run.
        let rejected: Result<usize, _> =
            application.execute(ApplicationCommand::RecoverObservedOrphanedRun {
                conversation_id: fixture.session.clone(),
                expected_run_id: "wrong-run".into(),
                host_invocation: binding.clone(),
                observed_session_lease: session_lease.clone(),
                observed_deliveries: vec![lease.clone()],
                interrupted_at: now.to_string(),
                interrupted_at_ms: now,
            });
        assert!(rejected.is_err());
        assert_eq!(
            serde_json::to_value(unchanged()).unwrap(),
            serde_json::to_value(&before).unwrap()
        );

        let mut changed_config = config.clone();
        changed_config.model_settings.endpoint = "https://changed.invalid/v1".into();
        let bad_binding = host_invocation_binding(&changed_config).unwrap();
        let rejected: Result<usize, _> =
            application.execute(ApplicationCommand::RecoverObservedOrphanedRun {
                conversation_id: fixture.session.clone(),
                expected_run_id: begun.begin.run_id.clone(),
                host_invocation: bad_binding,
                observed_session_lease: session_lease.clone(),
                observed_deliveries: vec![lease.clone()],
                interrupted_at: now.to_string(),
                interrupted_at_ms: now,
            });
        assert!(
            rejected.is_err(),
            "binding mismatch must be rejected inside the recovery transaction"
        );
        assert_eq!(
            serde_json::to_value(unchanged()).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        if let Some(current) = &session_lease {
            let mut stale = current.clone();
            stale.acquired_at_ms -= 1;
            let rejected: Result<usize, _> =
                application.execute(ApplicationCommand::RecoverObservedOrphanedRun {
                    conversation_id: fixture.session.clone(),
                    expected_run_id: begun.begin.run_id.clone(),
                    host_invocation: binding.clone(),
                    observed_session_lease: Some(stale),
                    observed_deliveries: vec![lease.clone()],
                    interrupted_at: now.to_string(),
                    interrupted_at_ms: now,
                });
            assert!(rejected.is_err());
            let retained: Option<nola_store_port::workspace_ownership::StoredSessionRunLease> =
                application
                    .execute(ApplicationCommand::GetSessionRunLease {
                        session_id: fixture.session.clone(),
                    })
                    .unwrap();
            assert_eq!(retained.as_ref(), Some(current));
            assert_eq!(
                serde_json::to_value(unchanged()).unwrap(),
                serde_json::to_value(&before).unwrap()
            );
        }

        assert_eq!(
            recover_observed_orphaned_deliveries_for_run(
                &mut application,
                &fixture.session,
                &begun.begin.run_id,
                &binding
            )
            .unwrap(),
            None
        );
        assert_eq!(
            unchanged().state,
            nola_domain::EffectDeliveryAttemptState::Interrupted,
            "the fixture really is recoverable when addressed by its exact Run"
        );
        let remaining: Option<nola_application::SessionRunLease> = application
            .execute(ApplicationCommand::GetSessionRunLease {
                session_id: fixture.session.clone(),
            })
            .unwrap();
        assert!(remaining.is_none());
    }

    #[test]
    fn host_binding_nonsecret_fallback_identity_and_old_run_hardcut() {
        let _environment = crate::store::test_env_lock();
        let (_fixture, mut config) = binding_config();
        config.runtime.openai_prompt_cache_key = Some("private-cache-key".into());
        config.runtime.openai_prompt_cache_key_prefix = Some("private-cache-prefix".into());
        config.runtime.llm_fallbacks =
            serde_json::from_value(serde_json::json!([{"endpoint":"https://fallback.invalid/v1"}]))
                .unwrap();
        let binding = host_invocation_binding(&config).unwrap();
        assert_eq!(
            http_route(&binding.fallbacks[0]).model,
            http_route(&binding.primary).model
        );
        let mut unadmitted = config.clone();
        unadmitted.runtime.admitted_max_retries = None;
        assert!(host_invocation_binding(&unadmitted)
            .unwrap_err()
            .contains("has not been admitted"));
        let mut options_changed = config.clone();
        options_changed.runtime.generation_options.seed = Some(0);
        options_changed.runtime.sampling_override = true;
        let generation_binding = host_invocation_binding(&options_changed).unwrap();
        assert_eq!(
            generation_binding
                .generation
                .generation_options
                .to_options()
                .seed,
            Some(0)
        );
        assert!(generation_binding.generation.sampling_override);
        assert_eq!(
            generation_binding.execution_policy_sha256, binding.execution_policy_sha256,
            "resolved generation is stored as data, not mixed into the capability policy digest"
        );
        assert!(binding.require_exact(&generation_binding).is_err());
        assert_eq!(
            http_route(&binding.fallbacks[0]).credential_sha256,
            http_route(&binding.primary).credential_sha256
        );
        let serialized = serde_json::to_string(&binding).unwrap();
        for secret in [
            "private-api-key",
            "query-secret",
            "private-cache-key",
            "private-cache-prefix",
        ] {
            assert!(!serialized.contains(secret));
        }
        assert_eq!(
            http_route(&binding.primary).endpoint,
            "https://provider.invalid/v1"
        );
        let identity = AgentTurnIdentity {
            host_invocation: Some(binding.clone()),
            ..Default::default()
        };
        validate_host_invocation_binding(&identity, &config).unwrap();
        let mut reasoning_changed = config.clone();
        reasoning_changed.runtime.reasoning_selection =
            Some(nola_core::agent::reasoning::ReasoningSelection::Effort {
                value: "low".into(),
            });
        assert!(validate_host_invocation_binding(&identity, &reasoning_changed).is_err());
        assert!(
            validate_host_invocation_binding(&AgentTurnIdentity::default(), &config)
                .unwrap_err()
                .contains("predates")
        );
        let mut changed = config.clone();
        changed.model_settings.api_key = "rotated-without-rebind".into();
        assert!(validate_host_invocation_binding(&identity, &changed).is_err());
        changed = config.clone();
        changed.model_settings.endpoint = "https://provider.invalid/v1?token=changed-secret".into();
        assert!(validate_host_invocation_binding(&identity, &changed).is_err());
        changed = config.clone();
        changed.runtime.temperature = Some(1.1);
        assert!(validate_host_invocation_binding(&identity, &changed).is_err());
        let skills = SkillRuntimeContext::from_configured_dirs(config.tool_settings.skill_dirs.clone());
        let changed_skills = SkillRuntimeContext::from_configured_dirs(changed.tool_settings.skill_dirs.clone());
        assert_ne!(
            core_agent_host_signature(&config, &skills, &[]).unwrap(),
            core_agent_host_signature(&changed, &changed_skills, &[]).unwrap(),
            "a new turn must rebuild its driver for generation changes"
        );
        changed = config.clone();
        changed.model_settings.endpoint = "https://user:private@provider.invalid/v1".into();
        assert!(host_invocation_binding(&changed).is_err());
        changed = config.clone();
        changed.runtime.llm_fallbacks[0].api_key = Some(String::new());
        assert_ne!(
            http_route(&host_invocation_binding(&changed).unwrap().fallbacks[0]).credential_sha256,
            http_route(&binding.primary).credential_sha256
        );
        let mut obsolete = binding;
        obsolete.version += 1;
        assert!(obsolete.validate().is_err());
    }

    #[test]
    fn host_binding_device_advertisement_requires_original_metadata_and_live_resolver() {
        let _environment = crate::store::test_env_lock();
        let (_fixture, config) = binding_config();
        let issued = now_epoch_ms().unwrap() as u64;
        let tool = crate::tools::DeviceBridgeRuntimeTool {
            schema: serde_json::json!({"type":"function","function":{"name":"device_write_file","parameters":{"type":"object","properties":{}}},"x_nola":{"target":"device"}}),
            capability: "files_write".into(),
            result_egress_receipt: nola_core::tool_policy::RuntimeResultEgressReceipt {
                tool_id: "device_write_file".into(),
                route: nola_core::tool_policy::ToolRoute::Device,
                manifest_id: "original-manifest".into(),
                host_id: "host".into(),
                client_id: "client".into(),
                session_id: "session".into(),
                issued_at_ms: issued,
                expires_at_ms: issued + 60_000,
                allow_result_egress: true,
            },
            result_egress_grant: nola_core::tool_policy::ResultEgressGrant {
                tool_id: "device_write_file".into(),
                route: nola_core::tool_policy::ToolRoute::Device,
                manifest_id: "original-manifest".into(),
                host_id: "host".into(),
                client_id: "client".into(),
                session_id: "session".into(),
                expires_at_ms: issued + 60_000,
            },
        };
        let advertisement = crate::tools::enter_device_bridge_tools(vec![tool.clone()]).unwrap();
        assert!(host_invocation_binding(&config)
            .unwrap_err()
            .contains("no active resolver"));
        let resolver = crate::tools::enter_device_bridge_resolver(
            nola_core::tool_policy::RuntimeIdentity {
                host_id: Some("host".into()),
                client_id: Some("client".into()),
                session_id: Some("session".into()),
            },
            Arc::new(|_, _| panic!("binding validation must never execute a Device tool")),
        )
        .unwrap();
        let identity = AgentTurnIdentity {
            host_invocation: Some(host_invocation_binding(&config).unwrap()),
            ..Default::default()
        };
        validate_host_invocation_binding(&identity, &config).unwrap();
        let mut refreshed = tool.clone();
        refreshed.result_egress_receipt.manifest_id = "refreshed-manifest".into();
        refreshed.result_egress_grant.manifest_id = "refreshed-manifest".into();
        refreshed.result_egress_receipt.issued_at_ms += 1;
        refreshed.result_egress_receipt.expires_at_ms += 1;
        refreshed.result_egress_grant.expires_at_ms += 1;
        {
            let _replacement =
                crate::tools::enter_device_bridge_tools(vec![refreshed.clone()]).unwrap();
            validate_host_invocation_binding(&identity, &config).unwrap();
        }
        for field in ["host", "client", "session", "schema", "capability"] {
            let mut changed = refreshed.clone();
            match field {
                "host" => {
                    changed.result_egress_receipt.host_id = "other".into();
                    changed.result_egress_grant.host_id = "other".into();
                }
                "client" => {
                    changed.result_egress_receipt.client_id = "other".into();
                    changed.result_egress_grant.client_id = "other".into();
                }
                "session" => {
                    changed.result_egress_receipt.session_id = "other".into();
                    changed.result_egress_grant.session_id = "other".into();
                }
                "schema" => {
                    changed.schema["function"]["description"] = serde_json::json!("changed")
                }
                _ => changed.capability = "other".into(),
            }
            let _replacement = crate::tools::enter_device_bridge_tools(vec![changed]).unwrap();
            assert!(
                validate_host_invocation_binding(&identity, &config).is_err(),
                "{field}"
            );
        }
        let mut changed = tool;
        changed.result_egress_grant.manifest_id = "different-manifest".into();
        {
            let _replacement = crate::tools::enter_device_bridge_tools(vec![changed]).unwrap();
            assert!(validate_host_invocation_binding(&identity, &config).is_err());
        }
        drop(resolver);
        assert!(validate_host_invocation_binding(&identity, &config)
            .unwrap_err()
            .contains("no active resolver"));
        drop(advertisement);
        assert!(validate_host_invocation_binding(&identity, &config).is_err());
    }

    #[test]
    fn real_host_permission_interruption_repeated_resume_preserves_original_call() {
        permission_entrypoint_fixture(false, false, false);
    }

    #[test]
    fn memory_revoked_while_permission_is_open_prevents_approved_file_write() {
        permission_entrypoint_fixture(true, false, false);
    }

    #[test]
    fn code_mode_real_nested_write_requires_permission_for_each_call() {
        permission_entrypoint_fixture(false, true, false);
    }

    #[test]
    fn code_mode_real_nested_network_requires_permission_for_each_call() {
        permission_entrypoint_fixture(false, true, true);
    }

    fn permission_entrypoint_fixture(revoke_during_permission: bool, nested: bool, network: bool) {
        // This in-process debug fixture nests provider, Host, Application and
        // repeated recovery frames on one thread. Declare its stack budget
        // locally rather than requiring a hidden RUST_MIN_STACK for cargo test.
        std::thread::Builder::new()
            .name("permission-recovery-fixture".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || permission_entrypoint_fixture_inner(revoke_during_permission, nested, network))
            .expect("spawn permission recovery fixture")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    }

    fn permission_entrypoint_fixture_inner(revoke_during_permission: bool, nested: bool, network: bool) {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-permission-entry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let fixture = Fixture {
            root: root.clone(),
            session: format!("permission-entry-{}", uuid::Uuid::new_v4()),
            environment: [
                "NOLA_HOME",
                "NOLA_DB_PATH",
                "NOLA_LLM_MAX_RETRIES",
                "NOLA_LONG_TERM_MEMORY_ENABLED",
            ]
            .into_iter()
            .map(|k| (k, std::env::var_os(k)))
            .collect(),
            stop: stop.clone(),
        };
        std::env::set_var("NOLA_HOME", &root);
        std::env::set_var("NOLA_DB_PATH", root.join("canonical.sqlite"));
        std::env::set_var("NOLA_LLM_MAX_RETRIES", "1");
        std::env::remove_var("NOLA_LONG_TERM_MEMORY_ENABLED");
        let target = root.join("approved-once.txt");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let upload_url = format!("http://{}/nested-upload", listener.local_addr().unwrap());
        let uploads = Arc::new(AtomicUsize::new(0));
        let server_uploads = uploads.clone();
        let posts = Arc::new(AtomicUsize::new(0));
        let count = posts.clone();
        let transient_failures = Arc::new(AtomicUsize::new(0));
        let failures = transient_failures.clone();
        let server_target = target.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(45);
            while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("accept fixture HTTP: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_string();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                assert!(length < 2_000_000);
                let mut input = vec![0; length];
                reader.read_exact(&mut input).unwrap();
                let (content_type, body) = if path == "/nested-upload" {
                    assert_eq!(input, b"fixture-payload");
                    server_uploads.fetch_add(1, Ordering::SeqCst);
                    ("text/plain", "received".to_string())
                } else if path == "/v1/models" {
                    (
                        "application/json",
                        serde_json::json!({"data":[{"id":"fixture-model","context_length":32768}]})
                            .to_string(),
                    )
                } else {
                    assert_eq!(path, "/v1/chat/completions");
                    let request: serde_json::Value = serde_json::from_slice(&input).unwrap();
                    assert!(
                        request.get("parallel_tool_calls").is_none(),
                        "capability support must not invent an explicit parallel request"
                    );
                    if !nested && count.load(Ordering::SeqCst) == 1
                        && failures.fetch_add(1, Ordering::SeqCst) == 0
                    {
                        write!(stream, "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                        continue;
                    }
                    let ordinal = count.fetch_add(1, Ordering::SeqCst);
                    assert!(
                        ordinal < 8,
                        "fixture exceeded recovery plus subsequent Turn sequence"
                    );
                    let delta = if ordinal == 0 && nested {
                        serde_json::json!({"tool_calls":[{"index":0,"id":"discover-code-call","type":"function","function":{"name":"tool_search","arguments":"{\"query\":\"select:code_mode_exec\"}"}}]})
                    } else if ordinal == 1 && nested {
                        let source = if network {
                            let args = serde_json::json!({"command":format!("curl --silent --show-error --max-time 5 --data fixture-payload {upload_url}"), "operation_kind":"execute", "cwd":server_target.parent().unwrap()});
                            format!("for (let i = 0; i < 2; i++) {{ text(await tools.bash({args})); }}")
                        } else {
                            format!("for (let i = 0; i < 2; i++) {{ text(await tools.write_file({{path: {}, content: 'write-' + i}})); }}", serde_json::to_string(&server_target).unwrap())
                        };
                        serde_json::json!({"tool_calls":[{"index":0,"id":"parent-code-call","type":"function","function":{"name":"code_mode_exec","arguments":serde_json::json!({"source":source,"initial_wait_ms":10000}).to_string()}}]})
                    } else if nested {
                        serde_json::json!({"content":"Nested permission fixture complete."})
                    } else if ordinal == 0 {
                        serde_json::json!({"tool_calls":[{"index":0,"id":"original-write-call","type":"function","function":{"name":"write_file","arguments":serde_json::json!({"path":server_target,"content":"approved exactly once"}).to_string()}}]})
                    } else if ordinal == 1 {
                        serde_json::json!({"tool_calls":[{"index":0,"id":"validate-write-call","type":"function","function":{"name":"bash","arguments":serde_json::json!({"command":format!("test -f {}", server_target.display()),"operation_kind":"validate","validation_targets":[server_target]}).to_string()}}]})
                    } else if ordinal >= 4 {
                        serde_json::json!({"content":"The subsequent turn is complete."})
                    } else if ordinal == 3 {
                        fixture_completion_audit(&input)
                    } else {
                        serde_json::json!({"tool_calls":[{"index":0,"id":"review-finished-call","type":"function","function":{"name":"request_completion_review","arguments":serde_json::json!({"candidate_answer":"Finished."}).to_string()}}]})
                    };
                    let first = serde_json::json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                    let last = serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":if ordinal < (if nested {2} else {3}) {"tool_calls"} else {"stop"}}]});
                    (
                        "text/event-stream",
                        format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"),
                    )
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let mut cli_config = crate::config::Config::from_args(vec![
            "nola".into(),
            "--session".into(),
            fixture.session.clone(),
            "--project".into(),
            root.to_string_lossy().into_owned(),
            "--endpoint".into(),
            endpoint,
            "--api-key".into(),
            "fixture".into(),
            "--model".into(),
            "fixture-model".into(),
        ]).unwrap();
        let mut config = cli_config.host_invocation_request().unwrap();
        config.session.save_session = true;
        config.session.supports_reviewed_text_parts = false;
        config.tool_settings.auto_approve = false;
        config.tool_settings.tools_enabled = true;
        config.tool_settings.tool_allowlist = Some(vec!["write_file".into(), "bash".into()]);
        if nested {
            config.tool_settings.tool_allowlist.as_mut().unwrap().extend(["tool_search".into(), "code_mode_exec".into(), "code_mode_wait".into()]);
            config.execution.max_iterations = 3;
        }
        config.tool_settings.allowed_dirs = vec![root.clone()];
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.runtime.memory_enabled = Some(revoke_during_permission);
        config
            .runtime
            .core_permission_rules
            .push(CorePermissionRule {
                capability_pattern: nola_core::agent::tool_permission_domain("write_file")
                    .expect("fixture must target the current Host write permission domain"),
                resource_pattern: "*".into(),
                decision: CoreRuleDecision::Ask,
                source: PermissionRuleSource::UserGlobal,
            });
        config
            .runtime
            .core_permission_rules
            .push(CorePermissionRule {
                capability_pattern: "bash".into(),
                resource_pattern: "*".into(),
                decision: if network { CoreRuleDecision::Ask } else { CoreRuleDecision::Allow },
                source: PermissionRuleSource::UserGlobal,
            });
        config.runtime.mac_launch = Some(mac_launch_fixture(&config));
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let mut emitter = Emitter::with_event_sink(move |event| sink.lock().unwrap().push(event));
        let resolver_calls = Arc::new(AtomicUsize::new(0));
        let trace_call_ids = Arc::new(Mutex::new(Vec::new()));
        let observed_calls = trace_call_ids.clone();
        let called = resolver_calls.clone();
        let permission_session = fixture.session.clone();
        // Both initial dispatch and recovered permission resolvers belong to
        // this canonical conversation, not an unbound ambient test thread.
        let _permission_session = crate::tools::enter_host_permission_session(&fixture.session);
        let guard = crate::tools::enter_host_permission_resolver(Arc::new(move |request, _, _| {
            if nested {
                if request.name == "code_mode_exec" {
                    return ToolPermissionResolution::Allow;
                }
                assert_eq!(request.name, if network { "bash" } else { "write_file" });
                let trace = crate::tools::take_pending_host_permission_trace(
                    &crate::tools::current_host_session_id().unwrap(), request,
                    &serde_json::Value::Object(request.args.clone()), "nested resource permission",
                ).expect("nested request must retain canonical permission trace");
                assert_eq!(trace["conversation_id"], permission_session);
                assert_eq!(trace["payload"]["permission_request_id"], request.id);
                assert!(trace["payload"]["call_id"].as_str().unwrap().starts_with("code-call-"));
                observed_calls.lock().unwrap().push(trace["payload"]["call_id"].as_str().unwrap().to_string());
                return if called.fetch_add(1, Ordering::SeqCst) == 0 {
                    ToolPermissionResolution::Allow
                } else {
                    ToolPermissionResolution::Deny
                };
            }
            assert_original_permission_trace(request, &observed_calls);
            called.fetch_add(1, Ordering::SeqCst);
            if revoke_during_permission {
                // Revoke after the execution-start check, while the actual
                // file tool is waiting for its human authorization response.
                std::env::set_var("NOLA_LONG_TERM_MEMORY_ENABLED", "false");
                ToolPermissionResolution::Allow
            } else {
                ToolPermissionResolution::TransportInterrupted(
                    nola_host_protocol::PermissionTransportInterruptionReason::Timeout,
                )
            }
        }));
        let host = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let outcome = host
            .lock()
            .unwrap()
            .run_turn_items(
                &config,
                host_user_input_items(&config, "write the file"),
                nola_core::context_engine::ContextPlatform::Unknown,
                Vec::new(),
                RuntimeExecutionControl::default(),
                &mut emitter,
            )
            .unwrap();
        if nested {
            fixture.stop.store(true, Ordering::SeqCst);
            server.join().unwrap();
            assert_eq!(resolver_calls.load(Ordering::SeqCst), 2, "each actual nested call requires its own authorization; outcome={outcome:?}, provider_requests={}, fixture events: {}",
                posts.load(Ordering::SeqCst), serde_json::to_string(&*events.lock().unwrap()).unwrap().chars().take(24000).collect::<String>());
            if network {
                assert_eq!(uploads.load(Ordering::SeqCst), 1, "denied second shell must not send another network request");
            } else {
                assert_eq!(std::fs::read_to_string(&target).unwrap(), "write-0", "denied second write must not alter the actual file");
            }
            let calls = trace_call_ids.lock().unwrap();
            assert_ne!(calls[0], calls[1]);
            let events = events.lock().unwrap();
            assert!(events.iter().any(|event| event["trace"]["kind"] == "tool.permission.resolved"
                && event["trace"]["payload"]["call_id"] == calls[0]
                && event["trace"]["payload"]["permission_decision"] == "allow_once"));
            assert!(events.iter().any(|event| event["trace"]["kind"] == "tool.completed"
                && event["trace"]["payload"]["call_id"] == calls[1]
                && event["trace"]["status"] == "failed"), "denied child must publish canonical failure");
            assert!(events.iter().any(|event| event["trace"]["kind"] == "tool.permission.resolved"
                && event["trace"]["payload"]["call_id"] == calls[1]
                && event["trace"]["payload"]["permission_decision"] == "deny"), "the second call must settle by denial, not budget exhaustion");
            assert!(outcome.is_terminal(), "bounded fixture must settle its Cell and Run");
            assert!(host.lock().unwrap().runtime.execution.permissions.pending.is_empty());
            return;
        }
        if revoke_during_permission {
            // Stop the isolated provider before assertions, so a failing
            // safety assertion never leaves its server thread running.
            fixture.stop.store(true, Ordering::SeqCst);
            server.join().unwrap();
            assert_eq!(resolver_calls.load(Ordering::SeqCst), 1);
            assert!(!target.exists(), "Allow must not execute a file write after Host authority was revoked while permission was open");
            assert!(outcome.is_terminal());
            let locked = host.lock().unwrap();
            assert_eq!(
                locked.runtime.execution.pending_effects.len(),
                0
            );
            assert!(locked.runtime.execution.permissions.pending.is_empty());
            drop(locked);
            let mut application =
                crate::application_port::CliApplicationPort::persistent_default().unwrap();
            assert!(application.active_run(&fixture.session).unwrap().is_none());
            assert_eq!(
                posts.load(Ordering::SeqCst),
                1,
                "revocation cannot issue the validation/candidate/audit model requests"
            );
            return;
        }
        assert!(!outcome.is_terminal());
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 1);
        let original = host.lock().unwrap().runtime.execution.permissions.pending.clone();
        let committed_binding = host
            .lock()
            .unwrap()
            .runtime
            .config
            .turn_identity
            .host_invocation
            .clone()
            .expect("the actual BeginTurn must retain binding after its configuration update");
        assert_eq!(committed_binding.generation.max_retries, 1);
        assert_eq!(host.lock().unwrap().memory_llm.max_retries, 1);
        std::env::set_var("NOLA_LLM_MAX_RETRIES", "0");
        let mut recovered_config = config.clone();
        recovered_config.runtime.memory_enabled = Some(true);
        freeze_host_output_admission(&mut recovered_config).unwrap();
        let frozen_memory = recovered_config
            .runtime
            .admitted_memory_policy
            .clone()
            .unwrap();
        assert!(!frozen_memory.long_term_enabled && !frozen_memory.automatic_extraction_enabled);
        let stored = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap()
            .load_checkpoint(&fixture.session)
            .unwrap()
            .unwrap();
        let checkpoint: CoreCheckpoint<AgentThreadState> =
            serde_json::from_str(&stored.checkpoint_json).unwrap();
        assert_eq!(checkpoint.state.configuration.memory_policy, frozen_memory);
        assert_eq!(
            committed_binding,
            host_invocation_binding(&recovered_config).unwrap()
        );
        assert_eq!(original.len(), 1);
        let canonical_call_id = original[0].call_id.as_str().to_string();
        assert_eq!(
            trace_call_ids.lock().unwrap().as_slice(),
            &[canonical_call_id.clone()]
        );
        assert!(!target.exists());
        let rejected = host
            .lock()
            .unwrap()
            .run_turn_items(
                &config,
                host_user_input_items(&config, "unrelated next turn"),
                nola_core::context_engine::ContextPlatform::Unknown,
                Vec::new(),
                RuntimeExecutionControl::default(),
                &mut emitter,
            )
            .unwrap_err();
        assert!(rejected.contains("permission response"), "{rejected}");
        assert_eq!(
            host.lock().unwrap().runtime.execution.permissions.pending,
            original
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        drop(guard);
        // Repeat on the cached Host, with a fresh resolver each time. No restored-once flag.
        let canonical_run_id = host
            .lock()
            .unwrap()
            .session_store
            .as_ref()
            .unwrap()
            .active_core_run_id()
            .unwrap()
            .to_string();
        assert!(resume_agent_core_expected_with_cancellation(
            &config,
            &mut emitter,
            RuntimeExecutionControl::default(),
            "wrong-run"
        )
        .is_err());
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        assert_eq!(
            host.lock().unwrap().runtime.execution.permissions.pending,
            original
        );
        for attempt in 0..2 {
            let observed_calls = trace_call_ids.clone();
            let called = resolver_calls.clone();
            let _resolver =
                crate::tools::enter_host_permission_resolver(Arc::new(move |request, _, _| {
                    assert_original_permission_trace(request, &observed_calls);
                    called.fetch_add(1, Ordering::SeqCst);
                    ToolPermissionResolution::TransportInterrupted(
                        nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
                    )
                }));
            if attempt == 0 {
                let mut changed_defaults = config.clone();
                changed_defaults.model_settings.endpoint = "http://127.0.0.1:9/changed-default".into();
                let result = resume_agent_core_expected_with_cancellation(
                    &changed_defaults,
                    &mut emitter,
                    RuntimeExecutionControl::default(),
                    &canonical_run_id,
                )
                .unwrap()
                .unwrap();
                assert!(!result.is_terminal());
            } else {
                // Product configuration and the composed Host request have
                // separate types. Carry only this fixture's explicit changes
                // back to the product entry; do not restore the old alias.
                cli_config.runtime.memory_enabled = config.runtime.memory_enabled;
                cli_config.runtime.core_permission_rules = config.runtime.core_permission_rules.clone();
                cli_config.runtime.mac_launch = config.runtime.mac_launch.clone();
                cli_config.save_session = config.session.save_session;
                cli_config.json_output = !config.session.supports_reviewed_text_parts;
                cli_config.tools_enabled = config.tool_settings.tools_enabled;
                cli_config.auto_approve = config.tool_settings.auto_approve;
                cli_config.tool_allowlist = config.tool_settings.tool_allowlist.clone();
                cli_config.allowed_dirs = config.tool_settings.allowed_dirs.clone();
                cli_config.skill_dirs = config.tool_settings.skill_dirs.clone();
                assert!(!crate::resume_selected_session_before_composer(&cli_config, &mut emitter).unwrap(), "ordinary CLI resume must stop before accepting a new prompt while the Run waits");
            }
            assert_eq!(
                host.lock().unwrap().runtime.execution.permissions.pending,
                original
            );
            assert_eq!(posts.load(Ordering::SeqCst), 1);
        }
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 3);
        drop(host);
        invalidate_core_agent_session(&fixture.session).unwrap();
        // Same model name cannot authorize a different provider endpoint on cold open.
        // Keep B listening to prove validation happens before even a provider probe.
        let forbidden_provider = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        forbidden_provider.set_nonblocking(true).unwrap();
        let mut changed_route = config.clone();
        changed_route.model_settings.endpoint = format!("http://{}/v1", forbidden_provider.local_addr().unwrap());
        // A takeover transport may be ephemeral. Resuming an explicitly named
        // persistent Run must still load that Run before validating its frozen
        // invocation binding.
        changed_route.session.save_session = false;
        assert_eq!(changed_route.model_settings.model, config.model_settings.model);
        let before_checkpoint =
            SessionStore::load_agent_checkpoint_for_session(&fixture.session).unwrap();
        let before_outbox = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap()
            .load_committed_host_requests(&canonical_run_id)
            .unwrap();
        let rejected = resume_agent_core_expected_with_cancellation(
            &changed_route,
            &mut emitter,
            RuntimeExecutionControl::default(),
            &canonical_run_id,
        )
        .unwrap_err();
        assert!(
            rejected.contains("Host invocation unavailable"),
            "{rejected}"
        );
        assert_eq!(
            forbidden_provider.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        let mut changed_credential = config.clone();
        changed_credential.model_settings.api_key = "different-credential".into();
        assert!(resume_agent_core_expected_with_cancellation(
            &changed_credential,
            &mut emitter,
            RuntimeExecutionControl::default(),
            &canonical_run_id
        )
        .unwrap_err()
        .contains("Host invocation unavailable"));
        assert_eq!(
            SessionStore::load_agent_checkpoint_for_session(&fixture.session).unwrap(),
            before_checkpoint
        );
        let after_outbox = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap()
            .load_committed_host_requests(&canonical_run_id)
            .unwrap();
        assert_eq!(
            serde_json::to_value(after_outbox).unwrap(),
            serde_json::to_value(before_outbox).unwrap()
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        assert!(!target.exists());
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 3);
        // Cold-open the same canonical DB with its original proven route/credentials;
        // explicit allow is the only tool-execution boundary.
        let observed_calls = trace_call_ids.clone();
        let called = resolver_calls.clone();
        let _resolver =
            crate::tools::enter_host_permission_resolver(Arc::new(move |request, _, _| {
                assert_original_permission_trace(request, &observed_calls);
                called.fetch_add(1, Ordering::SeqCst);
                ToolPermissionResolution::Allow
            }));
        let result = resume_agent_core_pending_with_cancellation(
            &recovered_config,
            &mut emitter,
            RuntimeExecutionControl::default(),
        )
        .unwrap()
        .unwrap();
        assert!(result.is_terminal());
        let completed_checkpoint =
            crate::application_port::CliApplicationPort::persistent_default()
                .unwrap()
                .load_checkpoint(&fixture.session)
                .unwrap()
                .unwrap();
        let completed_checkpoint: CoreCheckpoint<AgentThreadState> =
            serde_json::from_str(&completed_checkpoint.checkpoint_json).unwrap();
        assert_eq!(
            completed_checkpoint.state.configuration.memory_policy,
            frozen_memory
        );
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 4);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "approved exactly once"
        );
        assert_eq!(posts.load(Ordering::SeqCst), 4);
        assert_eq!(
            transient_failures.load(Ordering::SeqCst),
            2,
            "cold-restored Host must retry the one transient failure despite new env=0"
        );
        let completed = events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| {
                event["trace"]["kind"] == "tool.completed"
                    && event["trace"]["payload"]["call_id"].as_str()
                        == Some(canonical_call_id.as_str())
            })
            .count();
        assert_eq!(
            completed, 1,
            "the original tool action completes exactly once"
        );
        assert_eq!(
            trace_call_ids.lock().unwrap().as_slice(),
            &vec![canonical_call_id; 4]
        );
        assert!(resume_agent_core_pending_with_cancellation(
            &config,
            &mut emitter,
            RuntimeExecutionControl::default()
        )
        .unwrap()
        .is_none());
        assert_eq!(resolver_calls.load(Ordering::SeqCst), 4);
        // New submission reuses the same conversation but not its previous
        // client's false ceiling. Exercise the real outer admission reset.
        let mut next = recovered_config;
        next.runtime.memory_enabled = Some(true);
        next.tool_settings.tools_enabled = false;
        next.tool_settings.tool_allowlist = Some(Vec::new());
        // The recovered Run has now completed. This is a distinct caller read
        // for the next submission, including its visible transcript frontier.
        let next_host = acquire_core_agent_host(&next, Vec::new()).unwrap();
        let (revision, parent) = next_host.lock().unwrap().session_store.as_mut().unwrap()
            .read_submission_boundary().unwrap();
        next.session.expected_configuration_revision = Some(revision);
        next.session.expected_parent_message_id = parent;
        let result = run_agent_core_turn_with_execution(
            &next,
            Vec::new(),
            vec![AgentItem::user_message(
                ItemId::from_string("next-memory-user".into()),
                "Reply briefly",
            )],
            nola_core::context_engine::ContextPlatform::Unknown,
            Vec::new(),
            RuntimeExecutionControl::default(),
            &mut emitter,
        )
        .unwrap();
        assert!(result.is_terminal());
        let stored = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap()
            .load_checkpoint(&fixture.session)
            .unwrap()
            .unwrap();
        let next_checkpoint: CoreCheckpoint<AgentThreadState> =
            serde_json::from_str(&stored.checkpoint_json).unwrap();
        assert!(
            next_checkpoint
                .state
                .configuration
                .memory_policy
                .long_term_enabled
        );
        assert!(
            !next_checkpoint
                .state
                .configuration
                .memory_policy
                .automatic_extraction_enabled,
            "client true must not enable Host-default-disabled extraction"
        );
        assert_ne!(
            next_checkpoint
                .state
                .configuration
                .turn_identity
                .host_invocation
                .as_ref()
                .unwrap()
                .execution_policy_sha256,
            committed_binding.execution_policy_sha256
        );
        fixture.stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
    }
}
