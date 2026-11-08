// SPDX-License-Identifier: Apache-2.0
// Explicit transport recovery of an existing canonical Run; never a new input.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct HostRunResumeRequest {
    conversation_id: String,
    expected_run_id: String,
    expected_host_id: String,
    client_id: String,
}

fn host_run_resume_response(
    request: &mut Request,
    mobile: bool,
) -> Response<Box<dyn Read + Send>> {
    let token = if mobile {
        match mobile_bearer_token(request) {
            Some(token) => Some(token),
            None => return json_response(StatusCode(401), json!({"error":"unauthorized"})),
        }
    } else {
        None
    };
    let body = match token.as_deref() {
        Some(token) => read_mobile_json_value(request, token),
        None => read_json_value(request),
    };
    match body {
        Ok(body) => host_run_resume_response_body(body, token),
        Err(response) => response,
    }
}

fn host_run_resume_response_body(
    body: Value,
    token: Option<String>,
) -> Response<Box<dyn Read + Send>> {
    let input: HostRunResumeRequest = match serde_json::from_value(body) {
        Ok(input) => input,
        Err(error) => {
            return json_response(
                StatusCode(400),
                json!({"code":"invalid_resume_request","error":error.to_string()}),
            )
        }
    };
    if [
        &input.conversation_id,
        &input.expected_run_id,
        &input.expected_host_id,
        &input.client_id,
    ]
    .iter()
    .any(|value| value.trim().is_empty() || value.len() > 512)
    {
        return json_response(StatusCode(400), json!({"code":"invalid_resume_identity"}));
    }
    let host_id = host_identity().to_string();
    if input.expected_host_id != host_id {
        return json_response(
            StatusCode(409),
            json!({"code":"host_identity_changed","error":"explicit re-pairing required"}),
        );
    }
    let cancellation = RuntimeExecutionControl::default();
    let registration =
        match try_register_web_agent_run(&input.conversation_id, cancellation.clone()) {
            Ok(registration) => registration,
            Err(id) => {
                return json_response(
                    StatusCode(409),
                    json!({"code":"run_in_progress","activeExecutionId":id}),
                )
            }
        };
    // No body-derived model, context, input, submission ID, or replacement flag.
    let (tx, rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel();
    let heartbeat = spawn_sse_heartbeat(tx.clone(), stop_rx, StdDuration::from_secs(15));
    thread::spawn(move || {
        let observation_owner = (|| -> Result<ObservedWorkerOwner,String> {
            let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
            let active = observation_command(&mut application,json!({"type":"active_run","conversation_id":input.conversation_id}))?;
            if active.get("run_id").and_then(Value::as_str)!=Some(input.expected_run_id.as_str()) {return Err("resume Run changed".into());}
            let owner=ObservedWorkerOwner {conversation_id:input.conversation_id.clone(),run_id:input.expected_run_id.clone(),
                assistant_message_id:active.get("assistant_message_id").and_then(Value::as_str).ok_or("missing resume root")?.into(),
                client_id:input.client_id.clone(),generation:registration.execution_id};
            read_run_host_tool_plan(&HostToolPlanRead {
                expected_host_id: host_id.clone(), client_id: input.client_id.clone(),
                session_id: input.conversation_id.clone(), expected_run_id: input.expected_run_id.clone(),
                assistant_message_id: owner.assistant_message_id.clone(),
            })?;
            bind_observed_worker(&owner)?;
            Ok(owner)
        })();
        let _registration = registration;
        let _heartbeat_stop = stop_tx;
        let _heartbeat = heartbeat;
        let observation_owner = match observation_owner {Ok(owner)=>owner,Err(error)=>{
            let _=tx.send(mobile_sse_event(json!({"type":"error","error":error}),token.as_deref()));return;
        }};
        let _scope = enter_host_permission_session(&input.conversation_id);
        let resolver_tx = tx.clone();
        let resolver_token = token.clone();
        let resolver_host = host_id.clone();
        let resolver_client = input.client_id.clone();
        let resolver_session = input.conversation_id.clone();
        let resolver_run = input.expected_run_id.clone();
        let delegated_permissions = stream_delegated_permissions(observation_owner.clone(), tx.clone(), token.clone(), None);
        let journal_owner = observation_owner.clone();
        let resolver_owner = observation_owner;
        let _resolver = enter_host_permission_resolver(Arc::new(move |request, instructions, allow_conversation| {
            if bind_observed_worker(&resolver_owner).is_err() {
                return ToolPermissionResolution::TransportInterrupted(nola_host_protocol::PermissionTransportInterruptionReason::Unavailable);
            }
            wait_for_host_permission_in_run(
                request,
                &resolver_host,
                &resolver_client,
                &resolver_session,
                &resolver_tx,
                resolver_token.as_deref(),
                instructions,
                allow_conversation,
                StdDuration::from_secs(300),
                Some(&resolver_run),
            )
        }));
        let event_tx = tx.clone();
        let event_token = token.clone();
        let session = input.conversation_id.clone();
        let canonical_run_id = input.expected_run_id.clone();
        let mut journal_notifications = None;
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(move |mut event| {
            observe_journal_for_stream(&mut journal_notifications, &journal_owner, &event,
                &event_tx, &event_token, &host_id);
            if !mobile_event_is_canonical(&event) {
                return;
            }
            if event
                .get("trace")
                .and_then(|trace| serde_json::from_value::<AgentTraceEvent>(trace.clone()).ok())
                .is_some_and(|trace| trace.kind == AgentTraceKind::ToolPermissionRequested)
            {
                return;
            }
            if web_event_is_core_projection(&event) {
                event["source_host_id"] = json!(host_id);
                if event.get("type").and_then(Value::as_str) == Some("context_usage") {
                    event["protocol_version"] = json!(1);
                    event["session_id"] = json!(session);
                }
            }
            // The resume helper validates this Run against Application and
            // the locked Host before publishing or dispatching any effect.
            // A Run can span successive Core Turns; never rewrite trace IDs.
            event["canonical_run_id"] = json!(canonical_run_id);
            let _ = event_tx.send(mobile_sse_event(event, event_token.as_deref()));
        });
        let result = crate::core_agent_runtime::restore_exact_core_agent_run_config(
            &input.conversation_id, &input.expected_run_id,
        ).map_err(|error| match error {
            crate::core_agent_runtime::ColdCoreAgentRecoveryError::BlockedConfiguration { reason }
            | crate::core_agent_runtime::ColdCoreAgentRecoveryError::RetryableUnavailable { reason } => reason,
        }).and_then(|request| crate::core_agent_runtime::resume_agent_core_expected_with_cancellation(
            &request,
            &mut emitter,
            cancellation,
            &input.expected_run_id,
        ));
        match result {
            Ok(Some(_)) => {}
            Ok(None) => {
                let _ = tx.send(mobile_sse_event(json!({"type":"error","code":"no_active_run","message":"no canonical active Run to resume"}), token.as_deref()));
            }
            Err(error) => {
                let _ = tx.send(mobile_sse_event(
                    json!({"type":"error","code":"run_resume_failed","message":error}),
                    token.as_deref(),
                ));
            }
        }
        drop(delegated_permissions);
        let _ = tx.send(b"data: [DONE]\n\n".to_vec());
        drop(_heartbeat_stop);
        let _ = _heartbeat.join();
    });
    sse_response(rx)
}

#[cfg(test)]
mod host_run_resume_tests {
    use super::*;

    #[test]
    fn resume_handler_rejects_input_and_cannot_replace_same_session_execution() {
        let _lock = crate::store::test_env_lock();
        let directory =
            std::env::temp_dir().join(format!("nola-resume-handler-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        struct Cleanup {
            root: std::path::PathBuf,
            old: Option<std::ffi::OsString>,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                match &self.old {
                    Some(value) => std::env::set_var("NOLA_HOME", value),
                    None => std::env::remove_var("NOLA_HOME"),
                };
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        let _cleanup = Cleanup {
            root: directory.clone(),
            old: std::env::var_os("NOLA_HOME"),
        };
        std::env::set_var("NOLA_HOME", &directory);
        let session = format!("resume-{}", uuid::Uuid::new_v4());
        let base = json!({"conversation_id":session,"expected_run_id":"canonical-run","expected_host_id":host_identity(),"client_id":"fixture-client"});
        for key in [
            "prompt",
            "messages",
            "input",
            "context",
            "model",
            "replace_active_run",
            "submission_id",
        ] {
            let mut invalid = base.clone();
            invalid[key] = json!("forbidden");
            assert_eq!(
                host_run_resume_response_body(invalid, None).status_code(),
                StatusCode(400)
            );
        }
        let mut wrong = base.clone();
        wrong["expected_host_id"] = json!("other-host");
        assert_eq!(
            host_run_resume_response_body(wrong, None).status_code(),
            StatusCode(409)
        );
        let cancellation = RuntimeExecutionControl::default();
        let _active = try_register_web_agent_run(&session, cancellation.clone()).unwrap();
        assert_eq!(
            host_run_resume_response_body(base, None).status_code(),
            StatusCode(409)
        );
        assert!(!cancellation.is_requested());
        assert!(!directory.join("nola.db").exists());
    }
}

#[cfg(test)]
mod host_resume_success_tests {
    use super::*;
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    struct Fixture {
        root: std::path::PathBuf,
        session: String,
        environment: Vec<(&'static str, Option<std::ffi::OsString>)>,
        stop: Arc<AtomicBool>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);

            for (key, value) in &self.environment {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn real_resume_handler_reissues_nonce_and_approval_executes_original_once() {
        assert_resume_handler(false);
    }

    #[test]
    fn local_queued_turn_cannot_be_hijacked_by_predecessors_remote_client() {
        assert_resume_handler(true);
    }

    fn assert_resume_handler(successor_turn: bool) {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-permission-entry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let fixture = Fixture {
            root: root.clone(),
            session: format!("permission-entry-{}", uuid::Uuid::new_v4()),
            environment: ["NOLA_HOME", "NOLA_DB_PATH"]
                .into_iter()
                .map(|k| (k, std::env::var_os(k)))
                .collect(),
            stop: stop.clone(),
        };
        std::env::set_var("NOLA_HOME", &root);
        std::env::set_var("NOLA_DB_PATH", root.join("canonical.sqlite"));
        let target = root.join("approved-once.txt");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let posts = Arc::new(AtomicUsize::new(0));
        let count = posts.clone();
        let server_target = target.clone();
        let server_session = fixture.session.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + StdDuration::from_secs(45);
            while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(StdDuration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("accept fixture HTTP: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(StdDuration::from_secs(5)))
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
                let (content_type, body) = if path == "/v1/models" {
                    (
                        "application/json",
                        serde_json::json!({"data":[{"id":"fixture-model","context_length":32768}]})
                            .to_string(),
                    )
                } else {
                    assert_eq!(path, "/v1/chat/completions");
                    let ordinal = count.fetch_add(1, Ordering::SeqCst);
                    assert!(
                        ordinal < if successor_turn { 6 } else { 4 },
                        "permission recovery exceeded the reviewed completion sequence"
                    );
                    if successor_turn && ordinal == 0 {
                        crate::boundary_input_intent::execute(
                            &server_session,
                            crate::boundary_input_intent::InputOperation::NewSubmit("write the file in the next turn"),
                        )
                        .unwrap();
                    }
                    let write_response = ordinal == if successor_turn { 2 } else { 0 };
                    let validation_response = ordinal == if successor_turn { 3 } else { 1 };
                    let audit_response = ordinal == if successor_turn { 5 } else { 3 }
                        || (successor_turn && ordinal == 1);
                    let review_request = if successor_turn {
                        ordinal == 0 || ordinal == 4
                    } else { ordinal == 2 };
                    let delta = if write_response {
                        serde_json::json!({"tool_calls":[{"index":0,"id":"original-write-call","type":"function","function":{"name":"write_file","arguments":serde_json::json!({"path":server_target,"content":"approved exactly once"}).to_string()}}]})
                    } else if validation_response {
                        serde_json::json!({"tool_calls":[{"index":0,"id":"validate-write-call","type":"function","function":{"name":"bash","arguments":serde_json::json!({"command":format!("test -f {}", server_target.display()),"operation_kind":"validate","validation_targets":[server_target]}).to_string()}}]})
                    } else if review_request {
                        serde_json::json!({"tool_calls":[{"index":0,"id":format!("review-completion-{ordinal}"),"type":"function","function":{"name":"request_completion_review","arguments":serde_json::json!({"candidate_answer":"Finished."}).to_string()}}]})
                    } else if audit_response {
                        crate::core_agent_runtime::fixture_completion_audit(&input)
                    } else {
                        serde_json::json!({"content":"Finished."})
                    };
                    let first = serde_json::json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                    let last = serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":if write_response || validation_response || review_request {"tool_calls"} else {"stop"}}]});
                    (
                        "text/event-stream",
                        format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n"),
                    )
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let mut config = Config::from_args(vec![
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
        ])
        .unwrap();
        let original_submission = format!("original-submission-{}", uuid::Uuid::new_v4());
        config.host_submission_id = Some(original_submission.clone());
        let original_user=format!("{original_submission}:user");
        let original_assistant=format!("{original_submission}:assistant");
        config.host_user_message_id=Some(original_user.clone());
        config.host_assistant_message_id=Some(original_assistant.clone());
        // Exercise the same immutable request/digest/client registration as
        // Web admission. A later local CLI input does not inherit this binding.
        let mut submission_body = json!({
            "expected_host_id":host_identity(), "client_id":"resume-client",
            "conversationId":fixture.session, "submission_id":original_submission,
            "user_message_id":original_user, "assistant_message_id":original_assistant,
            "input_items":[{"text":"write the file"}]
        });
        submission_body["request_digest"] = json!(
            nola_domain::remote_submission::submission_request_digest(&submission_body).unwrap());
        let (_dispatch, registered_receipt, created) = register_stream_submission(&mut submission_body).unwrap();
        assert!(created);
        config.save_session = true;
        config.json_output = true;
        config.auto_approve = false;
        config.tools_enabled = true;
        config.tool_allowlist = Some(vec!["write_file".into(), "bash".into()]);
        config.allowed_dirs = vec![root.clone()];
        config.skill_dirs = Some(Vec::new());
        config
            .runtime
            .core_permission_rules
            .push(nola_core::agent::permissions::PermissionRule {
                capability_pattern: nola_core::agent::tool_permission_domain("write_file")
                    .expect("fixture must target the current Host write permission domain"),
                resource_pattern: "*".into(),
                decision: nola_core::agent::permissions::PermissionPolicyDecision::Ask,
                source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
            });
        config
            .runtime
            .core_permission_rules
            .push(nola_core::agent::permissions::PermissionRule {
                capability_pattern: "bash".into(),
                resource_pattern: "*".into(),
                decision: nola_core::agent::permissions::PermissionPolicyDecision::Allow,
                source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
            });

        config.remote_tool_plan = Some(host_tool_plan_prepare(HostToolPlanPrepare {
            expected_host_id: host_identity().to_owned(),
            client_id: "resume-client".into(),
            session_id: fixture.session.clone(),
        }, &config).unwrap());
        let (initial_tx, initial_rx) = mpsc::channel();
        let initial_session = fixture.session.clone();
        let initial_host = host_identity().to_string();
        let initial_permission_scope = enter_host_permission_session(&fixture.session);
        let resolver = enter_host_permission_resolver(Arc::new(move |request, instructions, allow_conversation| {
            wait_for_host_permission_with_timeout(
                request,
                &initial_host,
                "resume-client",
                &initial_session,
                &initial_tx,
                None,
                instructions,
                allow_conversation,
                StdDuration::ZERO,
            )
        }));
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        let outcome =
            crate::core_agent_runtime::run_agent_core(&config.host_invocation_request().unwrap(), "write the file", &mut emitter)
                .unwrap();
        assert!(!outcome.is_terminal());
        drop(resolver);
        drop(initial_permission_scope);
        let mut receipt_application=crate::application_port::CliApplicationPort::persistent_default().unwrap();
        let admitted:Option<nola_domain::remote_submission::SourceSubmissionReceipt>=receipt_application.execute(
            nola_application::ApplicationCommand::ReadSourceSubmissionReceipt {
                identity:registered_receipt.identity,request_digest:registered_receipt.request_digest,
            }).unwrap();
        assert_eq!(admitted.unwrap().phase,nola_domain::remote_submission::RemoteSubmissionPhase::Admitted);
        drop(receipt_application);
        let initial_frame =
            String::from_utf8(initial_rx.recv_timeout(StdDuration::from_secs(2)).unwrap()).unwrap();
        let initial_event: Value =
            serde_json::from_str(initial_frame.trim().strip_prefix("data: ").unwrap()).unwrap();
        let original_id = initial_event["id"].clone();
        let original_trace = initial_event["trace"].clone();
        let original_nonce = initial_event["nonce"].clone();
        let active = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap()
            .active_run(&fixture.session)
            .unwrap()
            .unwrap();
        let run_id = active.run_id;
        if successor_turn {
            assert_ne!(run_id, original_submission, "queued input owns a new Run");
            let predecessor: nola_domain::assistant_run_terminal::AssistantRunTerminalSeal =
                crate::application_port::CliApplicationPort::persistent_default().unwrap().execute(
                    nola_application::ApplicationCommand::LocalAssistantRunTerminalSeal {
                        conversation_id: fixture.session.clone(), run_id: original_submission.clone(),
                    }
                ).unwrap();
            assert_eq!(predecessor.identity.run_id, original_submission);
            assert_eq!(predecessor.result.run_id, original_submission);
        } else {
            assert_eq!(run_id, original_submission);
        }
        let checkpoint = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap()
            .load_checkpoint(&fixture.session)
            .unwrap()
            .unwrap();
        let before_checkpoint_json = checkpoint.checkpoint_json.clone();
        let checkpoint: nola_core::agent::recovery::CoreCheckpoint<
            nola_core::agent::AgentThreadState,
        > = serde_json::from_str(&checkpoint.checkpoint_json).unwrap();
        let core_turn = checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .turn_id
            .as_str()
            .to_string();
        assert_eq!(core_turn, run_id, "permission belongs to the active input's Run");
        if successor_turn {
            crate::core_agent_runtime::invalidate_core_agent_session(&fixture.session).unwrap();
        }
        let before_wrong_resume = serde_json::to_value(&checkpoint).unwrap();
        let before_posts = posts.load(Ordering::SeqCst);
        let before_outbox = serde_json::to_value(
            crate::application_port::CliApplicationPort::persistent_default()
                .unwrap()
                .load_committed_host_requests(&run_id)
                .unwrap(),
        )
        .unwrap();
        let wrong_body = json!({"conversation_id":fixture.session,"expected_run_id":"wrong-canonical-run","expected_host_id":host_identity(),"client_id":"resume-client"});
        let wrong_response = host_run_resume_response_body(wrong_body, None);
        let mut wrong_frames = String::new();
        wrong_response
            .into_reader()
            .read_to_string(&mut wrong_frames)
            .unwrap();
        assert!(wrong_frames.contains("error"), "{wrong_frames}");
        assert!(
            !wrong_frames.contains("permission_request"),
            "wrong Run must not issue a nonce"
        );
        assert_eq!(posts.load(Ordering::SeqCst), before_posts);
        assert_eq!(
            serde_json::to_value(
                crate::application_port::CliApplicationPort::persistent_default()
                    .unwrap()
                    .load_committed_host_requests(&run_id)
                    .unwrap()
            )
            .unwrap(),
            before_outbox
        );
        let after_wrong_resume = crate::application_port::CliApplicationPort::persistent_default()
            .unwrap()
            .load_checkpoint(&fixture.session)
            .unwrap()
            .unwrap();
        let after_wrong_resume_value = serde_json::to_value(
            serde_json::from_str::<nola_core::agent::recovery::CoreCheckpoint<
                nola_core::agent::AgentThreadState,
            >>(&after_wrong_resume.checkpoint_json)
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            after_wrong_resume_value,
            before_wrong_resume
        );
        let body = json!({"conversation_id":fixture.session,"expected_run_id":run_id,"expected_host_id":host_identity(),"client_id":"resume-client"});
        if !successor_turn {
            // The HTTP resume handler receives no startup model configuration.
            // Reconstruct the original route exclusively from durable admission.
            let restored = crate::core_agent_runtime::restore_exact_core_agent_run_config(
                &fixture.session, &run_id,
            ).unwrap();
            let original = config.host_invocation_request().unwrap().model_invocation_settings().unwrap();
            let recovered = restored.model_invocation_settings().unwrap();
            assert_eq!(recovered.endpoint(), original.endpoint());
            assert_eq!(recovered.model(), original.model());
        }
        let response = host_run_resume_response_body(body, None);
        assert_eq!(response.status_code(), StatusCode(200));
        if successor_turn {
            let mut frames = String::new();
            response.into_reader().read_to_string(&mut frames).unwrap();
            // A retained tool catalog does not confer remote ownership on a
            // local successor; its own admitted client receipt is required.
            assert!(frames.contains("Run has no admitted remote client binding"), "{frames}");
            assert!(!frames.contains("permission_request"), "unowned local Run must not issue remote authority");
            assert!(!frames.contains("assistant_run_terminal_committed_v1"));
            assert!(!target.exists(), "rejected resume must not execute the pending write");
            assert_eq!(posts.load(Ordering::SeqCst), before_posts);
            let mut application = crate::application_port::CliApplicationPort::persistent_default().unwrap();
            assert_eq!(application.active_run(&fixture.session).unwrap().unwrap().run_id, run_id);
            assert_eq!(serde_json::to_value(application.load_committed_host_requests(&run_id).unwrap()).unwrap(), before_outbox);
            let after = application.load_checkpoint(&fixture.session).unwrap().unwrap();
            assert!(after.checkpoint_json == before_checkpoint_json,
                "rejected remote resume must preserve the exact durable checkpoint bytes");
            fixture.stop.store(true, Ordering::SeqCst);
            server.join().unwrap();
            return;
        }
        let mut reader = std::io::BufReader::new(response.into_reader());
        let mut permission_count = 0;
        let mut terminal_projection = false;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let Some(data) = line.trim().strip_prefix("data: ") else {
                continue;
            };
            if data == "[DONE]" {
                break;
            }
            let event: Value = serde_json::from_str(data).unwrap();
            assert_ne!(event["type"], "error", "{event}");
            if event["type"] == "permission_request" {
                permission_count += 1;
                assert_eq!(event["canonical_run_id"], run_id);
                assert_eq!(event["id"], original_id);
                assert_eq!(
                    event["trace"]["payload"]["call_id"],
                    original_trace["payload"]["call_id"]
                );
                assert!(
                    event["trace"]["sequence"].as_u64().unwrap()
                        > original_trace["sequence"].as_u64().unwrap()
                );
                assert!(!event["nonce"].as_str().unwrap().is_empty());
                assert_ne!(event["nonce"], original_nonce);
                let decision = json!({"protocol_version":HOST_PERMISSION_PROTOCOL_VERSION,"nonce":event["nonce"],"request_id":event["id"],"host_id":event["host_id"],"client_id":event["client_id"],"session_id":event["session_id"],"tool_name":event["tool_name"],"resolution":{"type":"authorization","decision":"allow_once"}});
                submit_host_permission_decision(serde_json::from_value(decision.clone()).unwrap())
                    .unwrap();
                submit_host_permission_decision(serde_json::from_value(decision).unwrap()).unwrap();
            }
            if event["type"] == "assistant_run_terminal_committed_v1" {
                assert_eq!(event["canonical_run_id"], run_id);
                assert_eq!(event["run_id"], run_id);
                assert_eq!(event["seal_digest"].as_str().unwrap().len(), 64);
                let seal: nola_domain::assistant_run_terminal::AssistantRunTerminalSeal =
                    crate::application_port::CliApplicationPort::persistent_default().unwrap().execute(
                        nola_application::ApplicationCommand::LocalAssistantRunTerminalSeal {
                            conversation_id: fixture.session.clone(), run_id: run_id.clone(),
                        }
                    ).unwrap();
                assert_eq!(serde_json::to_value(&seal.seal_digest).unwrap(), event["seal_digest"]);
                assert_eq!(seal.protocol_version, 2);
                assert_eq!(seal.result.run_id, seal.identity.run_id);
                assert_eq!(seal.result.conversation_id, seal.identity.conversation_id);
                // This test proves exact permission resume and durable closure.
                // Writing a file is not by itself a requirement acceptance proof.
                terminal_projection = true;
            }
        }
        assert_eq!(permission_count, 1);
        assert!(terminal_projection);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "approved exactly once"
        );
        assert_eq!(
            posts.load(Ordering::SeqCst),
            if successor_turn { 6 } else { 4 }
        );
        assert!(
            crate::application_port::CliApplicationPort::persistent_default()
                .unwrap()
                .active_run(&fixture.session)
                .unwrap()
                .is_none()
        );
        fixture.stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
    }
}
