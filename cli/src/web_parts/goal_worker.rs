// SPDX-License-Identifier: Apache-2.0
// Wake adapter for Core-owned Goals. The durable Goal projection, lease and
// execution contracts survive this process; these handles only schedule work.
use crate::goal_control_port::{enqueue as enqueue_web_goal_control, apply_idle as apply_idle_web_goal_control};
struct WebGoalWorker {
    wake: std::sync::Condvar,
    stopped: std::sync::atomic::AtomicBool,
    signal: Mutex<bool>,
}
static WEB_GOAL_WORKER: OnceLock<Mutex<Option<Arc<WebGoalWorker>>>> = OnceLock::new();
struct WebGoalWorkerGuard(Arc<WebGoalWorker>);
impl Drop for WebGoalWorkerGuard {
    fn drop(&mut self) {
        self.0
            .stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.0.wake.notify_all();
    }
}
fn wake_web_goal_worker() {
    if let Some(worker) = WEB_GOAL_WORKER
        .get()
        .and_then(|slot| slot.lock().ok()?.clone())
    {
        if let Ok(mut signal) = worker.signal.lock() {
            *signal = true;
        }
        worker.wake.notify_one();
    }
}
fn start_web_goal_worker(config: &Config) -> Result<WebGoalWorkerGuard, String> {
    let worker = Arc::new(WebGoalWorker {
        wake: std::sync::Condvar::new(),
        stopped: std::sync::atomic::AtomicBool::new(false),
        signal: Mutex::new(true),
    });
    let mut slot = WEB_GOAL_WORKER
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "Goal worker registry unavailable")?;
    if slot
        .as_ref()
        .is_some_and(|old| !old.stopped.load(std::sync::atomic::Ordering::SeqCst))
    {
        return Err("Goal worker is already active".into());
    }
    *slot = Some(worker.clone());
    drop(slot);
    let state = worker.clone();
    let config = config.clone();
    std::thread::Builder::new()
        .name("web-goal-discovery".into())
        .spawn(move || {
            while !state.stopped.load(std::sync::atomic::Ordering::SeqCst) {
                if let Err(error) = scan_web_goals(&config) {
                    eprintln!("Goal discovery failed: {error}");
                }
                let signal = state.signal.lock().unwrap_or_else(|p| p.into_inner());
                let (mut signal, _) = state
                    .wake
                    .wait_timeout_while(signal, StdDuration::from_secs(2), |signal| !*signal)
                    .unwrap_or_else(|p| p.into_inner());
                *signal = false;
            }
        })
        .map_err(|error| format!("start Goal discovery: {error}"))?;
    Ok(WebGoalWorkerGuard(worker))
}
fn scan_web_goals(config: &Config) -> Result<(), String> {
    for due in crate::session::due_goal_sessions(crate::goal_runtime::goal_now_epoch_ms())? {
        if active_web_agent_runs()
            .lock()
            .map_err(|_| "worker registry unavailable")?
            .contains_key(&due.session_id)
        {
            continue;
        }
        // Controls accepted during a prior owner crash remain in the existing
        // canonical inbox. Reduce them before deciding whether to execute.
        crate::core_agent_runtime::drain_parent_agent_task_inbox(&due.session_id)?;
        // The Core due query and step own eligibility; a queued pause is
        // reconciled by step, never reinterpreted by this scheduler.
        let control = RuntimeExecutionControl::default();
        let registration = match try_register_web_agent_run(&due.session_id, control.clone()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let config = config.clone();
        std::thread::Builder::new()
            .name("web-goal-execution".into())
            .spawn(move || {
                let session = due.session_id.clone();
                let result = execute_web_goal(&config, &due, control, &registration);
                // This uses the existing control inbox reducer, not another queue
                // or Goal state machine. A pending Cancel wins over automatic pause.
                if let Err(error) =
                    crate::core_agent_runtime::drain_parent_agent_task_inbox(&session)
                {
                    eprintln!("Goal control reconciliation failed: {error}");
                }
                drop(registration);
                if let Err(error) = result {
                    eprintln!("Goal execution remains recoverable: {error}");
                }
            })
            .map_err(|error| format!("start Goal execution: {error}"))?;
    }
    Ok(())
}
fn web_goal_invocation(
    session: &str,
) -> Result<crate::host_invocation_request::HostInvocationRequest, String> {
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    if let Some(run) = application.active_run(session)? {
        return crate::core_agent_runtime::restore_exact_core_agent_run_config(
            session,
            &run.run_id,
        )
        .map_err(|error| match error {
            crate::core_agent_runtime::ColdCoreAgentRecoveryError::BlockedConfiguration {
                reason,
            }
            | crate::core_agent_runtime::ColdCoreAgentRecoveryError::RetryableUnavailable {
                reason,
            } => reason,
        });
    }
    let goal = crate::session::load_session_goal_projection(session)?
        .and_then(|projection| projection.active)
        .ok_or("Goal execution requires an admitted Goal binding")?;
    crate::goal_runtime::restore_goal_execution_request(session, &goal.id)
}

// Only a new Goal may resolve the user's current explicit conversation
// selection. Existing Goals restore their own immutable execution binding.
fn new_web_goal_invocation(
    config: &Config,
    session: &str,
) -> Result<crate::host_invocation_request::HostInvocationRequest, String> {
    let body = apply_conversation_request_configuration(json!({"conversationId":session}))?;
    let resolved = resolve_llm_settings(&body, config, None, Some(session)).map_err(|_| {
        "Goal model configuration is unavailable; select a model in this conversation"
    })?;
    let mut config = conversation_execution_config(config, resolved, session);
    let store = NolaStore::open_default()?;
    let settings = load_agent_settings_from_store(&store)?;
    let profile = permission_profile_for_request(&body, &settings)?;
    apply_agent_tool_settings(&mut config, &settings);
    config.permission_profile = Some(profile);
    config.auto_approve = profile.auto_approve();
    config.network_policy = profile.network_policy().into();
    config
        .runtime
        .core_permission_rules
        .extend(web_permission_rules_for_core(
            &load_state()?.permission_rules,
        ));
    let skills = web_skill_runtime_for_settings(config.skill_dirs.as_deref(), &settings)?;
    config.skill_dirs = skills.configured_dirs().map(<[PathBuf]>::to_vec);
    let (tools, _) = capture_current_host_tool_plan_tools(&config, session)?;
    config.tool_allowlist = Some(tools.into_iter().map(|tool| tool.tool_id).collect());
    config.tools_enabled = config
        .tool_allowlist
        .as_ref()
        .is_some_and(|tools| !tools.is_empty());
    config.host_input_origin = nola_core::agent::AgentInputOrigin::ScheduledAction;
    config.host_submission_id = None;
    config.host_user_message_id = None;
    config.host_assistant_message_id = None;
    config.host_invocation_request()
}
fn block_web_goal_configuration(session: &str) -> Result<(), String> {
    let projection =
        crate::session::load_session_goal_projection(session)?.ok_or("Goal projection missing")?;
    let goal = projection.active.as_ref().ok_or("Goal missing")?;
    let command_id = format!("goal-block:{}:{}", goal.id, projection.revision);
    let input = nola_core::agent::AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope {
            operation_id: command_id.clone(),
            command: GoalRuntimeCommand::GuardedControl {
                expected_goal_id: goal.id.clone(),
                expected_control_token: projection.control_token().expect("active Goal control token"),
                control: Box::new(GoalRuntimeCommand::BlockExecution {
                    now_epoch_ms: crate::goal_runtime::goal_now_epoch_ms(),
                }),
            },
        },
    };
    crate::agent_command_port::enqueue_agent_command(
        session,
        &command_id,
        &serde_json::to_string(&input).map_err(|error| error.to_string())?,
        crate::goal_runtime::goal_now_epoch_ms(),
    )?;
    crate::core_agent_runtime::drain_parent_agent_task_inbox(session)?;
    Ok(())
}

fn execute_web_goal(
    _config: &Config,
    due: &crate::session::DueGoalSession,
    control: RuntimeExecutionControl,
    registration: &WebAgentRunRegistration,
) -> Result<(), String> {
    let mut request = match web_goal_invocation(&due.session_id) {
        Ok(request) => request,
        Err(error) => {
            block_web_goal_configuration(&due.session_id)?;
            return Err(error);
        }
    };
    if let Some(active) = crate::application_port::CliApplicationPort::persistent_default()?
        .active_run(&due.session_id)?
    {
        if let Some(assistant) = active.assistant_message_id {
            bind_web_goal_worker(
                &due.session_id,
                &active.run_id,
                &assistant,
                registration.execution_id,
            )?;
        }
    }
    let _scope = enter_host_permission_session(&due.session_id);
    let generation = registration.execution_id;
    let (permission_tx, permission_rx) = mpsc::channel::<Vec<u8>>();
    // The existing permission transport records sanitized challenges against
    // the exact observed worker. Drain its delivery channel without rebuilding
    // a second permission observation or retaining private trace bodies.
    let permission_events = std::thread::spawn(move || for _ in permission_rx {});
    let permission_session = due.session_id.clone();
    let permission_control = control.clone();
    let _resolver = enter_host_permission_resolver(Arc::new(
        move |request, instructions, allow_conversation| {
            if permission_control.is_requested() {
                return ToolPermissionResolution::TransportInterrupted(
                    nola_host_protocol::PermissionTransportInterruptionReason::Disconnected,
                );
            }
            let active = crate::application_port::CliApplicationPort::persistent_default()
                .and_then(|mut app| app.active_run(&permission_session))
                .ok()
                .flatten();
            wait_for_host_permission_in_run(
                request,
                host_identity(),
                "web-goal",
                &permission_session,
                &permission_tx,
                None,
                instructions,
                allow_conversation,
                StdDuration::from_secs(300),
                active.as_ref().map(|run| run.run_id.as_str()),
            )
        },
    ));
    let session = due.session_id.clone();
    let mut output =
        crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(move |event| {
            if event.get("type").and_then(Value::as_str) == Some("submission_admitted") {
                if let (Some(run), Some(assistant)) = (
                    event.get("run_id").and_then(Value::as_str),
                    event.get("assistant_message_id").and_then(Value::as_str),
                ) {
                    if let Err(error) = bind_web_goal_worker(&session, run, assistant, generation) {
                        eprintln!("Goal Run observation binding failed: {error}");
                    }
                }
            }
        });
    let result = crate::goal_runtime::run_scheduled_host_goal(
        &mut request,
        &due.objective,
        &due.constraints.join("\n"),
        &mut output,
        control,
    )
    .map(|_| ());
    drop(_resolver);
    let _ = permission_events.join();
    result
}
fn web_goal_stop_committed(session: &str, run: &str) -> Result<bool, String> {
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    if app
        .active_run(session)?
        .as_ref()
        .map(|active| active.run_id.as_str())
        != Some(run)
    {
        return Ok(false);
    }
    let Some(stored) = app.load_checkpoint(session)? else {
        return Ok(false);
    };
    let checkpoint: nola_core::agent::recovery::CoreCheckpoint<nola_core::agent::AgentThreadState> =
        serde_json::from_str(&stored.checkpoint_json).map_err(|error| error.to_string())?;
    Ok(checkpoint
        .state
        .goal_projection
        .execution_owner(run)
        .is_some()
        && checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .is_some_and(|turn| turn.status == nola_core::agent::AgentTurnStatus::Cancelling))
}

fn bind_web_goal_worker(
    session: &str,
    run: &str,
    assistant: &str,
    generation: u64,
) -> Result<(), String> {
    let projection =
        crate::session::load_session_goal_projection(session)?.ok_or("Goal projection missing")?;
    projection
        .execution_owner(run)
        .ok_or("Run is not owned by a Core Goal contract")?;
    let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
    let admission = observation_command(
        &mut application,
        json!({"type":"local_run_message_admission_head", "conversation_id":session,"run_id":run}),
    )?;
    if admission
        .pointer("/identity/assistant_message_id")
        .and_then(Value::as_str)
        != Some(assistant)
    {
        return Err("Goal Run admission identity mismatch".into());
    }
    let runs = active_web_agent_runs()
        .lock()
        .map_err(|_| "Goal worker registry unavailable")?;
    if !runs
        .get(session)
        .is_some_and(|control| control.execution_id() == generation)
    {
        return Err("Goal execution generation changed".into());
    }
    OBSERVED_WORKERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "Goal observation registry unavailable")?
        .insert(
            generation,
            ObservedWorkerOwner {
                conversation_id: session.into(),
                run_id: run.into(),
                assistant_message_id: assistant.into(),
                client_id: "web-goal".into(),
                generation,
            },
        );
    Ok(())
}

#[cfg(test)]
mod web_goal_worker_tests {
    use super::*;
    use std::io::{BufRead, Write};
    #[test]
    fn existing_conversation_goal_admits_real_http_run_and_guarded_pause() {
        exercise_web_goal_stop(false, false, false);
    }
    #[test]
    fn goal_permission_challenge_is_observable_and_stops_only_after_committed_interrupt() {
        exercise_web_goal_stop(true, false, false);
    }
    #[test]
    fn paused_goal_executes_second_http_round_with_its_frozen_catalog() {
        exercise_web_goal_stop(false, true, false);
    }
    #[test]
    fn goal_budget_blocker_stops_after_one_canonical_run() {
        exercise_web_goal_stop(false, false, true);
    }
    fn exercise_web_goal_stop(permission_wait: bool, resume_execution: bool, budget_blocked: bool) {
        let _lock = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        struct Env(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for Env {
            fn drop(&mut self) {
                for (key, value) in &self.0 {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
            }
        }
        let _env = Env(["NOLA_HOME", "NOLA_DB_PATH"]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect());
        std::env::set_var("NOLA_HOME", root.path());
        std::env::set_var("NOLA_DB_PATH", root.path().join("product.sqlite"));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let (started_tx, started_rx) = mpsc::channel();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_stop = stop.clone();
        let target = root.path().join("permission-required.txt");
        let server_target = target.clone();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + StdDuration::from_secs(50);
            let mut streams = Vec::new();
            while !server_stop.load(std::sync::atomic::Ordering::SeqCst)
                && std::time::Instant::now() < deadline
            {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(StdDuration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(StdDuration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    if let Some(v) = header.to_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if line.contains("/models ") {
                    let body = r#"{"data":[{"id":"goal-fixture"}]}"#;
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                    continue;
                }
                assert!(line.contains("/chat/completions "), "{line}");
                let payload: Value = serde_json::from_slice(&body).unwrap();
                assert!(payload.to_string().contains("Summarize a result"));
                eprintln!("fixture request: model={} stream={} tools={} goal_tool={}", payload["model"], payload["stream"], payload["tools"].as_array().map_or(0, Vec::len), payload["tools"].as_array().is_some_and(|tools|tools.iter().any(|tool|tool.pointer("/function/name").and_then(Value::as_str)==Some("goal_complete"))));
                if permission_wait {
                    let delta = json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"goal-write","type":"function","function":{"name":"write_file","arguments":json!({"path":server_target,"content":"requires explicit approval"}).to_string()}}]},"finish_reason":null}]});
                    let end =
                        json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
                    let body = format!("data: {delta}\n\ndata: {end}\n\ndata: [DONE]\n\n");
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                    started_tx.send(()).unwrap();
                    continue;
                }
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
                stream.flush().unwrap();
                started_tx.send(()).unwrap();
                let stopped = server_stop.clone();
                streams.push(std::thread::spawn(move || {
                    stream.set_read_timeout(Some(StdDuration::from_millis(50))).unwrap();
                    while !stopped.load(std::sync::atomic::Ordering::SeqCst)
                        && std::time::Instant::now() < deadline
                    {
                        match stream.peek(&mut [0_u8; 1]) {
                            Ok(0) => break,
                            Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {},
                            Err(_) => break,
                            _ => {},
                        }
                    }
                    let _ = stream.write_all(b"0\r\n\r\n");
                }));
            }
            for stream in streams { stream.join().unwrap(); }
        });
        let mut config = Config::from_args([
            "run".into(),
            "--project".into(),
            root.path().to_string_lossy().into_owned(),
            "--endpoint".into(),
            endpoint.clone(),
            "--model".into(),
            "goal-fixture".into(),
            "--api-key".into(),
            "fixture".into(),
        ])
        .unwrap();
        config.skill_dirs = Some(Vec::new());
        if permission_wait {
            config.runtime.core_permission_rules.push(nola_core::agent::permissions::PermissionRule {
                capability_pattern: nola_core::agent::tool_permission_domain("write_file").unwrap(), resource_pattern: "*".into(),
                decision: nola_core::agent::permissions::PermissionPolicyDecision::Ask,
                source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
            });
        }
        NolaStore::open_default().unwrap().upsert_model_profile(nola_application::ModelProfile {
            id:"goal-profile".into(), display_name:"Fixture".into(), model_name:"goal-fixture".into(),
            provider:"openai-compatible".into(), runtime:"remote".into(),
            source_json:json!({"baseURL":endpoint,"endpointID":"goal-endpoint","wireProtocol":"chat_completions",
                "endpointEnabled":true,"modelEnabled":true,"capabilities":["chat"],
                "executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":true}}).to_string(),
            parameters_json:json!({"temperature":0.7,"topP":0.9,"maxOutputTokens":if budget_blocked {1024} else {4096},"maxContextLength":if budget_blocked {4096} else {32768}}).to_string(),
            selected:true, enabled:true, revision:1, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        }).unwrap();
        let session = "web-goal-existing";
        let conversation =
            WebConversation::new(session.into(), "Goal".into(), now_string(), now_string());
        NolaStore::open_default()
            .unwrap()
            .create_web_conversation(&json!(conversation))
            .unwrap();
        let request = new_web_goal_invocation(&config, session).unwrap();
        let _supervisor =
            crate::runtime_effect_supervisor::CliRuntimeEffectSupervisor::start_if_persistent(
                &request,
            )
            .unwrap();
        let start_envelope = nola_application::goal_coordinator::create_goal_command(
            "web-test", "Summarize a result", "", crate::goal_runtime::goal_now_epoch_ms());
        crate::core_agent_runtime::submit_agent_core_goal_command(&request, start_envelope.clone()).unwrap();
        let frozen: Option<nola_application::goal_execution::GoalExecutionBinding> =
            crate::application_port::CliApplicationPort::persistent_default().unwrap().execute(
                nola_application::ApplicationCommand::LoadGoalExecutionBinding {
                    conversation_id: session.into(), goal_id: "goal-web-test".into(),
                }).unwrap();
        assert_eq!(frozen.unwrap().template.core_config.host_catalog.unwrap().capabilities["glob"].exposure,
            nola_core::agent::host::HostCapabilityExposure::Deferred,
            "Goal Start must freeze the same execution-tier discovery surface as its first dispatch");
        // Start atomically admits the first canonical Run with its frozen
        // execution binding, before any provider dispatch.
        assert!(crate::application_port::CliApplicationPort::persistent_default()
            .unwrap().active_run(session).unwrap().is_some());
        let before_first_run = web_goal_invocation(session).unwrap();
        assert_eq!(before_first_run.model_settings.endpoint, request.model_settings.endpoint);
        assert_eq!(before_first_run.model_settings.model, request.model_settings.model);
        assert_eq!(before_first_run.runtime.selected_profile, request.runtime.selected_profile);
        let control = RuntimeExecutionControl::default();
        let registration = try_register_web_agent_run(session, control.clone()).unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let mut changed_defaults = config.clone();
        changed_defaults.tools_enabled = false;
        changed_defaults.tool_allowlist = Some(Vec::new());
        changed_defaults.network_policy = "deny".into();
        let worker_config = changed_defaults.clone();
        let worker = std::thread::spawn(move || {
            let due = crate::session::DueGoalSession {
                session_id: session.into(),
                objective: "Summarize a result".into(),
                constraints: vec![],
            };
            let result = execute_web_goal(&worker_config, &due, control, &registration);
            done_tx.send(result).unwrap();
        });
        if budget_blocked {
            done_rx.recv_timeout(StdDuration::from_secs(20)).expect("canonical budget blocker must finish the worker").unwrap();
            worker.join().unwrap();
            let projection = crate::session::load_session_goal_projection(session).unwrap().unwrap();
            let goal = projection.active.unwrap();
            assert_eq!(goal.status, GoalStatus::NeedsUser);
            assert_eq!(goal.contracts.len(), 1, "a Core blocked result must not loop into another Run");
            assert!(started_rx.try_recv().is_err(), "oversized mandatory input must not reach the provider");
            assert!(crate::application_port::CliApplicationPort::persistent_default().unwrap().active_run(session).unwrap().is_none());
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
            server.join().unwrap();
            return;
        }
        if started_rx.recv_timeout(StdDuration::from_secs(20)).is_err() {
            let result = done_rx.try_recv();
            let goal = crate::session::load_session_goal_projection(session).unwrap();
            stop.store(true, std::sync::atomic::Ordering::SeqCst);
            let state = goal.and_then(|p| p.active).map(|g| (g.status, g.stop_reason, g.contracts.len()));
            let retained = root.keep();
            panic!("actual model request must start; worker={result:?}; goal={state:?}; evidence={}", retained.display());
        }
        if resume_execution {
            let before_retry = crate::application_port::CliApplicationPort::persistent_default().unwrap()
                .active_run(session).unwrap().unwrap();
            crate::core_agent_runtime::submit_agent_core_goal_command(&request, start_envelope.clone()).unwrap();
            let after_retry = crate::application_port::CliApplicationPort::persistent_default().unwrap()
                .active_run(session).unwrap().unwrap();
            assert_eq!(before_retry.run_id, after_retry.run_id, "Start retry must not replace an active Run");
        }
        if permission_wait {
            let deadline = Instant::now() + StdDuration::from_secs(10);
            loop {
                let mut app =
                    crate::application_port::CliApplicationPort::persistent_default().unwrap();
                let observed = read_run_observation(
                    &mut app,
                    host_identity(),
                    RunObservationRequest::Discover {
                        conversation_id: session.into(),
                        expected_host_id: host_identity().into(),
                        client_id: "test-browser".into(),
                    },
                );
                if let Ok(value) = &observed {
                    if value
                        .pointer("/result/permissions/0/client_id")
                        .and_then(Value::as_str)
                        == Some("web-goal")
                    {
                        break;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "exact Goal permission challenge must be observable: {observed:?}"
                );
                std::thread::sleep(StdDuration::from_millis(25));
            }
        }
        // The control route uses this same helper. Changed process defaults
        // must not replace the exact policy of the already admitted Goal Run.
        let mut restored = web_goal_invocation(session).unwrap();
        assert!(restored.tool_settings.tools_enabled);
        assert!(restored.tool_settings.tool_allowlist.as_ref().is_some_and(|tools| !tools.is_empty()));
        let binding = crate::core_agent_runtime::fixture_host_invocation_binding(&mut restored).unwrap();
        let checkpoint = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap().load_checkpoint(session).unwrap().unwrap();
        let checkpoint: nola_core::agent::recovery::CoreCheckpoint<nola_core::agent::AgentThreadState> =
            serde_json::from_str(&checkpoint.checkpoint_json).unwrap();
        checkpoint.state.configuration.turn_identity.host_invocation.as_ref().unwrap()
            .require_exact(&binding).unwrap();
        if !permission_wait {
            // Exercise the actual HTTP handler while the model owns the Host
            // mutex. A control must return a durable queue receipt promptly;
            // only the Core operation ledger proves it was applied.
            let projection = crate::session::load_session_goal_projection(session).unwrap().unwrap();
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let url = format!("http://{}/goal", server.server_addr());
            let control_config = changed_defaults.clone();
            let handler = std::thread::spawn(move || {
                let mut request = server.recv().unwrap();
                let response = control_conversation_goal_response(&mut request, session, &control_config);
                request.respond(response).unwrap();
            });
            let started = Instant::now();
            let response = reqwest::blocking::Client::builder().timeout(StdDuration::from_secs(3))
                .build().unwrap().post(url).json(&json!({
                    "expectedHostId":host_identity(), "operationId":"fixture-resume-control", "action":"resume", "expectedGoalId":projection.active.as_ref().unwrap().id,
                    "expectedControlToken":projection.control_token().unwrap()
                })).send().unwrap();
            assert_eq!(response.status().as_u16(), 202);
            let receipt: Value = response.json().unwrap();
            assert_eq!(receipt["authoritative"], false);
            assert_eq!(receipt["controlPending"], true);
            assert!(started.elapsed() < StdDuration::from_secs(3));
            handler.join().unwrap();
            let operation = receipt["operationId"].as_str().unwrap();
            let deadline = Instant::now() + StdDuration::from_secs(5);
            loop {
                let projection = crate::session::load_session_goal_projection(session).unwrap().unwrap();
                if let Some(committed) = projection.applied_operations.get(operation) {
                    // This Goal is already running. Transport acceptance
                    // must not turn Core's semantic rejection into success.
                    assert_eq!(committed.rejection.as_deref(), Some("Goal is not resumable"));
                    break;
                }
                assert!(Instant::now() < deadline, "queued Resume needs a committed Core receipt");
                std::thread::sleep(StdDuration::from_millis(25));
            }
        }
        let projection = crate::session::load_session_goal_projection(session)
            .unwrap()
            .unwrap();
        let active = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap()
            .active_run(session)
            .unwrap()
            .unwrap();
        assert!(projection.execution_owner(&active.run_id).is_some());
        let envelope = GoalRuntimeCommandEnvelope {
            operation_id: "pause-live-goal".into(),
            command: GoalRuntimeCommand::GuardedControl {
                expected_goal_id: projection.active.as_ref().unwrap().id.clone(),
                expected_control_token: projection.control_token().expect("active Goal control token"),
                control: Box::new(GoalRuntimeCommand::Pause {
                    status: GoalStatus::Paused,
                    wake_at_epoch_ms: None,
                    now_epoch_ms: crate::goal_runtime::goal_now_epoch_ms(),
                }),
            },
        };
        // Cold recovery does not reconstruct the Web-only observer registry.
        // The same committed Goal execution must remain controllable.
        active_web_agent_runs().lock().unwrap().remove(session);
        assert!(crate::goal_control_port::enqueue(session, envelope).unwrap());
        let result = done_rx.recv_timeout(StdDuration::from_secs(10));
        if result.is_err() {
            let mut app = crate::application_port::CliApplicationPort::persistent_default().unwrap();
            let checkpoint = app.load_checkpoint(session).unwrap().unwrap();
            let state: Value = serde_json::from_str(&checkpoint.checkpoint_json).unwrap();
            eprintln!("Goal pause diagnostic {}", json!({"turn_id":state.pointer("/state/execution/active_turn/turn_id"),"turn_status":state.pointer("/state/execution/active_turn/status"),"goal":state.pointer("/state/goal_projection/active/status"),"operation":state.pointer("/state/goal_projection/applied_operations/pause-live-goal"),"contract":state.pointer("/state/goal_projection/active/contracts/0/execution_run_id")}));
        }
        if !resume_execution { stop.store(true, std::sync::atomic::Ordering::SeqCst); }
        worker.join().unwrap();
        result
            .expect("pause must interrupt a pending model without waiting for its response")
            .unwrap();
        let projection = crate::session::load_session_goal_projection(session)
            .unwrap()
            .unwrap();
        assert!(
            !target.exists(),
            "pause must not authorize the requested write"
        );
        // A delayed background wake can carry the original Goal Run
        // descriptor after the user's Pause already committed. It is not a
        // fresh interactive Resume and must leave the guarded token unchanged.
        assert_eq!(restored.input.host_input_origin, request.input.host_input_origin,
            "restoration uses the Goal Start template, not the last Run descriptor");
        restored.input.host_input_origin = nola_core::agent::AgentInputOrigin::GoalInteractiveUser;
        let before_background = projection.clone();
        let mut quiet = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        let background = crate::goal_runtime::run_scheduled_host_goal(&mut restored,
            "Summarize a result", "", &mut quiet, RuntimeExecutionControl::default());
        assert!(background.unwrap_err().contains("Goal requires explicit resume before claim"));
        let after_background = crate::session::load_session_goal_projection(session).unwrap().unwrap();
        assert_eq!(after_background, before_background, "background restore cannot manufacture a user Resume");
        let paused = projection.active.unwrap();
        assert_eq!(paused.status, GoalStatus::Paused);
        assert_eq!(
            paused.contracts.last().unwrap().status,
            nola_core::agent::GoalContractStatus::Interrupted
        );
        let mut next_contract = paused.contracts.last().unwrap().clone();
        next_contract.round += 1;
        next_contract.id = format!("{}:round:{}", paused.id, next_contract.round);
        next_contract.execution_run_id = None;
        next_contract.status = nola_core::agent::GoalContractStatus::Planned;
        crate::core_agent_runtime::submit_agent_core_goal_command(
            &request,
            GoalRuntimeCommandEnvelope {
                operation_id: "resume-confirmed-pause".into(),
                command: GoalRuntimeCommand::Resume {
                    now_epoch_ms: crate::goal_runtime::goal_now_epoch_ms(),
                },
            },
        )
        .unwrap();
        crate::core_agent_runtime::submit_agent_core_goal_command(
            &request,
            GoalRuntimeCommandEnvelope {
                operation_id: "next-round-after-confirmed-stop".into(),
                command: GoalRuntimeCommand::PlanContract {
                    contract: next_contract,
                    now_epoch_ms: crate::goal_runtime::goal_now_epoch_ms(),
                },
            },
        )
        .unwrap();
        assert!(
            crate::application_port::CliApplicationPort::persistent_default()
                .unwrap()
                .active_run(session)
                .unwrap()
                .is_none()
        );
        if resume_execution {
            let second_control = RuntimeExecutionControl::default();
            let second_registration = try_register_web_agent_run(session, second_control.clone()).unwrap();
            let (second_tx, second_rx) = mpsc::channel();
            let second_worker = std::thread::spawn(move || {
                let due = crate::session::DueGoalSession {session_id:session.into(),objective:"Summarize a result".into(),constraints:vec![]};
                second_tx.send(execute_web_goal(&config, &due, second_control, &second_registration)).unwrap();
            });
            if started_rx.recv_timeout(StdDuration::from_secs(20)).is_err() {
                let result = second_rx.try_recv();
                stop.store(true, std::sync::atomic::Ordering::SeqCst);
                let retained = root.keep();
                panic!("resumed Goal must reach frozen provider; worker={result:?}; evidence={}", retained.display());
            }
            let next = crate::application_port::CliApplicationPort::persistent_default().unwrap().active_run(session).unwrap().unwrap();
            assert_ne!(next.run_id, active.run_id);
            assert_eq!(next.assistant_message_id, active.assistant_message_id, "Goal rounds retain their canonical assistant root");
            let latest = crate::session::load_session_goal_projection(session).unwrap().unwrap();
            assert!(latest.execution_owner(&next.run_id).is_some());
            assert!(crate::goal_control_port::enqueue(session, GoalRuntimeCommandEnvelope {
                operation_id:"pause-second-actual-round".into(),
                command:GoalRuntimeCommand::GuardedControl {
                    expected_goal_id:latest.active.as_ref().unwrap().id.clone(),expected_control_token:latest.control_token().unwrap(),
                    control:Box::new(GoalRuntimeCommand::Pause {status:GoalStatus::Paused,wake_at_epoch_ms:None,
                        now_epoch_ms:crate::goal_runtime::goal_now_epoch_ms()}),
                },
            }).unwrap());
            second_rx.recv_timeout(StdDuration::from_secs(10)).expect("second pause must stop the actual provider request").unwrap();
            second_worker.join().unwrap();
            // Reopen the real SQLite repository after both provider rounds. A
            // root's current message.run_id must not erase the previous Run.
            let mut cold = crate::application_port::CliApplicationPort::persistent_default().unwrap();
            let history: Vec<nola_application::TraceRun> = cold.execute(
                nola_application::ApplicationCommand::TraceRunsForConversation { conversation_id: session.into() }
            ).unwrap();
            for run in [&active, &next] {
                let trace = history.iter().find(|trace| trace.run_id == run.run_id)
                    .expect("each actual Goal Run remains in cold formal history");
                assert_eq!(trace.message_id, run.assistant_message_id);
                let replay: serde_json::Value = cold.execute(nola_application::ApplicationCommand::LoadExactRunTraceReplay {
                    conversation_id:session.into(),run_id:run.run_id.clone(),
                    assistant_message_id:run.assistant_message_id.clone().unwrap(),
                    after_sequence:0,through_sequence:None,limit:256,
                }).unwrap();
                assert!(replay.is_object());
            }

            assert!(crate::application_port::CliApplicationPort::persistent_default().unwrap().active_run(session).unwrap().is_none());
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        server.join().unwrap();
    }
}
