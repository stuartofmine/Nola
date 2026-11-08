// SPDX-License-Identifier: Apache-2.0
// Same SQLite Run + production driver + real OS process stop. No model/network.
    #[cfg(unix)]
    #[test]
    fn real_external_shell_stop_same_run_resume_uses_store_authority() {
        use nola_core::agent::host::{HostResponse, HostResponseEnvelope};
        let _environment = crate::store::test_env_lock();
        let (fixture, mut config) = binding_config();
        config.session.session_id = Some(fixture.session.clone());
        config.session.save_session = true;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.tools_enabled = true;
        config.tool_settings.auto_approve = true;
        config.tool_settings.permission_profile = Some(crate::permissions::PermissionProfile::AllowAll);
        config.tool_settings.tool_allowlist = Some(vec!["bash".into()]);
        config.tool_settings.force_tool_set = "all".into();
        let pid_path = fixture.root.join("started.pid");
        let late_path = fixture.root.join("must-not-exist");
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
                "call_id":"external-shell", "name":"bash", "arguments":{
                    "command":"printf '%s' $$ > started.pid; sleep 30; printf late > must-not-exist",
                    "cwd":fixture.root,"operation_kind":"execute","execution_mode":"foreground","timeout_sec":10
                }
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
        // Match the production pump's dispatch_prepared boundary before dispatch.
        host.lock().unwrap().runtime.driver_mut().bind_file_read_run(&run);
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
        let abort = host.lock().unwrap().runtime.driver_mut().effect_delivery_abort_handle();
        let effect = original.effect_id.0.clone();
        let ready = pid_path.clone();
        let stopper = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !ready.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            abort(&effect);
            assert!(ready.exists(), "actual shell must start before targeted abort");
        });
        let mut returned_stop = None;
        let dispatch = nola_core::agent::host::AgentHostDriver::dispatch(
            host.lock().unwrap().runtime.driver_mut(), original.clone(), &mut |response| {
                if let HostResponse::ToolCompleted { output, .. } = &response.payload {
                    let result: nola_host_protocol::tool_result::ToolResult = serde_json::from_value(output.clone()).unwrap();
                    let receipt = result.execution.unwrap();
                    assert_eq!(receipt.state, nola_host_protocol::tool_result::ExecutionState::Unknown);
                    assert!(receipt.checks.is_empty());
                    returned_stop = receipt.local_stop;
                    // Physical return precedes a disconnected business-result delivery.
                    // Keep the actual Core ExecuteTool pending for subsequent Cancel.
                    return Err("fixture drops business response after real process stop".into());
                }
                app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                    conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest.clone(),
                    received_at_ms: nola_runtime_driver::monotonic_epoch_millis(), response,
                }).map(|_| ())
            });
        let stopped = stopper.join();
        assert!(stopped.is_ok(), "shell did not start: {dispatch:?}");
        if let Err(error) = dispatch { assert!(error.contains("fixture drops business response"), "{error}"); }
        let pid: i32 = std::fs::read_to_string(&pid_path).unwrap().parse().unwrap();
        let mut status = 0;
        assert_eq!(unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) }, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD));
        assert!(!late_path.exists());
        {
            let stop = returned_stop.expect("production bash must return typed physical stop evidence");
            assert_eq!(stop.scope, nola_domain::execution_evidence::LocalExecutionStopScope::OwnedProcessGroup { leader_pid: pid as u32 });
            assert_eq!(stop.identity.effect_id, original.effect_id.0);
            assert_eq!(stop.identity.request_id, original.request_id);
        }
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        host.lock().unwrap().commit_core_submission("real-shell-cancel", AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "real-shell-cancel".into(), reason: Some("fixture".into()),
        }, &mut emitter).unwrap();
        let now = nola_runtime_driver::monotonic_epoch_millis();
        let controls = app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: run.clone(), worker_generation: worker.worker_generation.clone(), lease_owner: worker.lease_owner.clone(),
            claimed_at_ms: now, lease_expires_at_ms: now + 60000, limit: 64,
        }).unwrap();
        let control = controls.requests.iter().find(|request| matches!(&request.payload,
            nola_core::agent::host::HostRequest::Cancel { target_effect_id, .. } if target_effect_id == &original.effect_id)).unwrap().clone();
        host.lock().unwrap().runtime.driver_mut().begin_resumed_turn(&run, RuntimeExecutionControl::default(), Default::default());
        let mut confirmed = false;
        nola_core::agent::host::AgentHostDriver::dispatch(host.lock().unwrap().runtime.driver_mut(), control.clone(), &mut |response| {
            confirmed |= matches!(response.payload, HostResponse::CancellationConfirmed { .. });
            app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                conversation_id: fixture.session.clone(), run_id: run.clone(), invocation_config_digest: digest.clone(),
                received_at_ms: nola_runtime_driver::monotonic_epoch_millis(), response,
            }).map(|_| ())
        }).unwrap();
        assert!(confirmed, "production driver and Store must accept the same Run physical stop");
        host.lock().unwrap().runtime.driver_mut().begin_resumed_turn("foreign-run", RuntimeExecutionControl::default(), Default::default());
        let mut foreign = control;
        foreign.turn_id = Some("foreign-run".into());
        foreign.delivery.as_mut().unwrap().attempt_id.push_str("-foreign");
        let mut foreign_confirmed = false;
        assert!(nola_core::agent::host::AgentHostDriver::dispatch(host.lock().unwrap().runtime.driver_mut(), foreign,
            &mut |response| { foreign_confirmed |= matches!(response.payload, HostResponse::CancellationConfirmed { .. }); Ok(()) }).is_err());
        assert!(!foreign_confirmed);
        assert!(!late_path.exists());
    }
