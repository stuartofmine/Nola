// SPDX-License-Identifier: Apache-2.0
// Test modules extracted from external_executor.rs; included at the same module scope.
#[cfg(test)]
mod tests {
    #[test]
    fn codex_transport_is_owned_stdio_not_shared_daemon_proxy() {
        assert_eq!(super::codex_app_server_arguments(), ["app-server", "--listen", "stdio://"]);
    }
    #[test]
    fn codex_terminal_requires_explicit_success_status() {
        for status in [Value::Null, serde_json::json!("interrupted"), serde_json::json!("inProgress"), serde_json::json!("failed")] {
            let events = parse_codex_app_server_message(&serde_json::json!({
                "method":"turn/completed", "params":{"turn":{"status":status}}
            }));
            assert!(matches!(events.as_slice(), [ExternalExecutorEvent::Failed { .. }]));
        }
        let events = parse_codex_app_server_message(&serde_json::json!({
            "method":"turn/completed", "params":{"turn":{"status":"completed"}}
        }));
        assert!(matches!(events.as_slice(), [ExternalExecutorEvent::Completed { .. }]));
    }
    #[cfg(unix)]
    #[test]
    fn codex_initialize_deadline_reaps_owned_process_tree() {
        let _environment = crate::store::test_env_lock();
        let previous = std::env::var_os("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS");
        std::env::set_var("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS", "1");
        let (config, descriptor) = protocol_fixture(ExecutionBackendKind::Codex,
            "while IFS= read -r line; do :; done\nwait");
        let mut observer = TestExternalObserver::default();
        let result = run_codex_app_server(&config, "fixture", true, &mut observer, None, &descriptor);
        match previous {
            Some(value) => std::env::set_var("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS", value),
            None => std::env::remove_var("NOLA_EXTERNAL_EXECUTION_SETUP_SECONDS"),
        }
        assert!(result.unwrap_err().contains("initialize handshake timed out"));
        assert_protocol_fixture_reaped(&config);
        fs::remove_dir_all(config.project_root).unwrap();
    }
    use super::*;

    #[derive(Default)]
    struct TestExternalObserver {
        cancel_on_rejected_text: Option<RuntimeExecutionControl>,
        reject_session: bool,
        cancel_on_session: Option<RuntimeExecutionControl>,
        stop_confirmed: bool,
        approve_permission: bool,
        reject_permission: bool,
        reject_permission_write: bool,
        permission_writes: Vec<(String, String)>,
        permissions: Vec<(String, Value)>,
    }

    impl ExternalExecutionObserver for TestExternalObserver {
        fn external_text(&mut self, _: &str) -> Result<(), String> {
            if let Some(control) = self.cancel_on_rejected_text.as_ref() {
                control.request(); return Err("fixture output commit rejected".into());
            }
            Ok(())
        }
        fn external_reasoning(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn external_session_started(&mut self, _: &str, _: &str) -> Result<(), String> {
            if let Some(control) = &self.cancel_on_session { control.request(); }
            if self.reject_session {
                Err("fixture admission rejected".into())
            } else {
                Ok(())
            }
        }
        fn external_permission_requested(
            &mut self,
            request_id: &str,
            call_id: &str,
            _: &str,
            arguments: &Value,
            _: &str,
        ) -> Result<Option<bool>, String> {
            self.permissions
                .push((request_id.into(), arguments.clone()));
            if let Some(expected) = arguments
                .pointer("/toolCall/toolCallId")
                .and_then(Value::as_str)
            {
                assert_eq!(call_id, expected);
            }
            if self.reject_permission {
                return Err("fixture claim admission rejected".into());
            }
            Ok(self.approve_permission.then_some(true))
        }
        fn external_permission_written(
            &mut self,
            request_id: &str,
            call_id: &str,
        ) -> Result<(), String> {
            self.permission_writes
                .push((request_id.into(), call_id.into()));
            if self.reject_permission_write {
                Err("fixture written admission rejected".into())
            } else {
                Ok(())
            }
        }
        fn external_tool_started(&mut self, _: &str, _: &str, _: &Value) -> Result<(), String> {
            Ok(())
        }
        fn external_tool_finished(
            &mut self,
            _: &str,
            _: &str,
            _: &Value,
            _: &str,
            _: bool,
        ) -> Result<(), String> {
            Ok(())
        }
        fn external_context_usage(&mut self, _: usize, _: usize) -> Result<(), String> {
            Ok(())
        }
        fn external_execution_cancelled(&mut self, _: &str) -> Result<(), String> {
            self.stop_confirmed = true;
            Ok(())
        }
        fn external_execution_completed(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[cfg(unix)]
    fn protocol_fixture(
        kind: ExecutionBackendKind,
        body: &str,
    ) -> (ExternalExecutionSettings, ExecutionBackendDescriptor) {
        protocol_fixture_with_shutdown(kind, body, true)
    }

    #[cfg(unix)]
    fn protocol_fixture_with_shutdown(
        kind: ExecutionBackendKind, body: &str, exits_on_eof: bool,
    ) -> (ExternalExecutionSettings, ExecutionBackendDescriptor) {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "nola-delegated-protocol-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let executable = directory.join("executor");
        // Persistent protocol fixtures close on EOF and reap their own child,
        // like a well-behaved server; waiting on a 30s test sleeper after EOF
        // would test an uncooperative shutdown rather than this protocol.
        let body = body.trim_end();
        let body = if exits_on_eof { body.strip_suffix("\nwait").unwrap_or(body) } else { body };
        // A concurrent fork in another test can inherit a briefly open writer
        // and make Linux execve report ETXTBSY. Write in an isolated child so
        // unrelated test children cannot inherit the executable's write handle.
        let mut writer = Command::new("/bin/sh").args(["-c", "cat > \"$1\"", "fixture-writer"])
            .arg(&executable).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        writer.stdin.take().unwrap().write_all(format!("#!/bin/sh\nif [ \"$2\" = daemon ]; then exit 0; fi\necho $$ > process.pid\nsleep 30 &\nowned_child=$!\necho $owned_child > descendant.pid\ntrap 'kill \"$owned_child\" 2>/dev/null; wait \"$owned_child\" 2>/dev/null' EXIT\n{body}\n").as_bytes()).unwrap();
        assert!(writer.wait().unwrap().success());
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let mut descriptor = ExecutionBackendDescriptor::built_in_nola();
        descriptor.id = match kind {
            ExecutionBackendKind::Codex => "codex",
            ExecutionBackendKind::OpenCode => "opencode",
            _ => "claude_code",
        }
        .into();
        descriptor.kind = kind;
        descriptor.executable_path = Some(executable.to_string_lossy().into_owned());
        // A protocol fixture has no reason to load the developer's daily
        // configuration or credentials, even when its executable is local.
        let config = ExternalExecutionSettings {
            project_root: directory, execution_backend: descriptor.id.clone(),
            external_execution_session_id: Some("original-session".into()),
            external_execution_model: None, external_execution_model_provider: None,
            host_context_components: Vec::new(), host_attachments: Vec::new(),
            budget: Default::default(), permission_profile: crate::permissions::PermissionProfile::Cautious,
            host_input_origin: nola_core::agent::AgentInputOrigin::InteractiveUser,
        };
        (config, descriptor)
    }

    #[cfg(unix)]
    fn assert_protocol_fixture_reaped(config: &ExternalExecutionSettings) {
        for file in ["process.pid", "descendant.pid"] {
            let pid: i32 = fs::read_to_string(config.project_root.join(file))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "{file} remained alive");
        }
    }

    #[cfg(unix)]
    #[test]
    fn delegated_cancel_after_session_stops_owned_processes_before_ack() {
        for kind in [ExecutionBackendKind::ClaudeCode, ExecutionBackendKind::OpenCode, ExecutionBackendKind::Codex] {
          for during_setup in [true, false] {
            let body = if during_setup { "while IFS= read -r line; do :; done\nwait" } else if kind == ExecutionBackendKind::ClaudeCode {
                r#"while IFS= read -r line; do
  case "$line" in
    *'"subtype":"initialize"'*) echo '{"type":"control_response","response":{"subtype":"success","request_id":"nola-initialize","response":{}}}' ;;
    *'"type":"user"'*) echo '{"type":"system","subtype":"init","session_id":"original-session"}' ;;
  esac
done
wait"#
            } else {
                r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"id":2,"result":{}}' ;;
    *'"method":"thread/resume"'*) echo '{"id":2,"result":{"thread":{"id":"original-session"}}}' ;;
    *'"method":"session/prompt"'*|*'"method":"turn/start"'*) echo dispatched_after_stop > forbidden ;;
  esac
done
wait
"#
            };
            let (config, descriptor) = protocol_fixture(kind, body);
            let control = RuntimeExecutionControl::default();
            let setup_cancel = during_setup.then(|| {
                let control = control.clone();
                let marker = config.project_root.join("descendant.pid");
                std::thread::spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !marker.exists() && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(5)); }
                    control.request();
                })
            });
            let mut observer = TestExternalObserver { cancel_on_session: (!during_setup).then(|| control.clone()), ..Default::default() };
            let result = match kind {
                ExecutionBackendKind::ClaudeCode => run_stream_json_process(&config, "fixture", true, &mut observer, Some(control), &descriptor),
                ExecutionBackendKind::OpenCode => run_opencode_acp(&config, "fixture", true, &mut observer, Some(control), &descriptor),
                _ => run_codex_app_server(&config, "fixture", true, &mut observer, Some(control), &descriptor),
            };
            if let Some(cancel) = setup_cancel { cancel.join().unwrap(); }
            assert_eq!(result.unwrap_err(), "external execution cancelled");
            assert!(observer.stop_confirmed);
            assert_protocol_fixture_reaped(&config);
            assert!(!config.project_root.join("forbidden").exists(), "cancelled session dispatched a new turn");
            fs::remove_dir_all(config.project_root).unwrap();
          }
        }
    }

    #[cfg(unix)]
    #[test]
    fn blocked_external_stdin_write_is_cancellable_and_reaps_its_processes() {
        let (config, descriptor) = protocol_fixture_with_shutdown(ExecutionBackendKind::ClaudeCode,
            "ps -o pgid= -p $$ > process.pgid\ntouch ready\nwait", false);
        let control = RuntimeExecutionControl::default();
        let worker_control = control.clone();
        let root = config.project_root.clone();
        let (started, ready) = mpsc::sync_channel(1);
        let (finished, result) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let child = external_command(Path::new(descriptor.executable_path.as_ref().unwrap()))
                .current_dir(&config.project_root).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
            let mut child = ManagedExternalChild::new(child);
            let mut stdin = child.stdin.take();
            let mut monitor = ExternalExecutionBudgetMonitor::new(ExternalExecutionBudget::from_config(&config), Arc::new(AtomicU64::new(0)));
            let mut observer = TestExternalObserver::default();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !config.project_root.join("ready").exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            let pid = child.id();
            let pgid = unsafe { libc::getpgid(pid as i32) };
            assert!(config.project_root.join("ready").exists(), "fixture did not become ready: child_pid={pid} pgid={pgid} status={:?}", child.try_wait());
            let reported_pid = fs::read_to_string(config.project_root.join("process.pid")).unwrap();
            let reported_pgid = fs::read_to_string(config.project_root.join("process.pgid")).unwrap();
            eprintln!("blocked-input fixture ready: child_pid={pid} actual_pgid={pgid} script_pid={} script_pgid={}", reported_pid.trim(), reported_pgid.trim());
            assert_eq!(reported_pid.trim().parse::<u32>().unwrap(), pid);
            assert_eq!(reported_pgid.trim().parse::<i32>().unwrap(), pgid);
            assert_eq!(pgid, pid as i32, "fixture must own its process group before testing a group stop");
            started.send(pid).unwrap();
            let outcome = write_external_input(&mut stdin, vec![b'x'; 2 * 1024 * 1024], &mut child,
                Some(&worker_control), &mut monitor, &mut observer, "claude_code");
            finished.send((outcome, observer.stop_confirmed)).unwrap();
        });
        let pid = ready.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(result.recv_timeout(Duration::from_millis(100)), Err(mpsc::RecvTimeoutError::Timeout)), "non-consuming child unexpectedly accepted a 2MiB write");
        control.request();
        let (outcome, confirmed) = result.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|error| {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
            panic!("blocked input did not respond to cancellation: {error}");
        });
        worker.join().unwrap();
        assert_eq!(outcome.unwrap_err(), "external execution cancelled");
        assert!(confirmed);
        // Both PIDs were recorded before the non-consuming shell entered wait.
        for file in ["process.pid", "descendant.pid"] {
            let pid: i32 = fs::read_to_string(root.join(file)).unwrap().trim().parse().unwrap();
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "blocked writer left {file} alive");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn delegated_strict_resume_protocol_failure_never_starts_a_new_session() {
        for kind in [ExecutionBackendKind::OpenCode, ExecutionBackendKind::Codex] {
            let (config, descriptor) = protocol_fixture(
                kind.clone(),
                r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*|*'"method":"session/resume"'*|*'"method":"thread/resume"'*) echo '{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"unknown original session"}}' ;;
    *'"method":"session/new"'*|*'"method":"thread/start"'*) echo fallback > forbidden ;;
  esac
done
wait
"#,
            );
            let mut observer = TestExternalObserver::default();
            let error = match kind {
                ExecutionBackendKind::OpenCode => {
                    run_opencode_acp(&config, "fixture", true, &mut observer, None, &descriptor)
                }
                _ => {
                    run_codex_app_server(&config, "fixture", true, &mut observer, None, &descriptor)
                }
            }
            .unwrap_err();
            assert!(error.contains("unknown original session"), "{error}");
            let requests = fs::read_to_string(config.project_root.join("requests.jsonl")).unwrap();
            assert!(requests.contains("original-session"));
            assert!(
                !requests.contains("session/new") && !requests.contains("thread/start"),
                "{requests}"
            );
            assert!(!config.project_root.join("forbidden").exists());
            assert_protocol_fixture_reaped(&config);
            fs::remove_dir_all(config.project_root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn delegated_observer_rejection_stops_all_real_protocol_processes() {
        for kind in [
            ExecutionBackendKind::ClaudeCode,
            ExecutionBackendKind::OpenCode,
            ExecutionBackendKind::Codex,
        ] {
            let body = if kind == ExecutionBackendKind::ClaudeCode {
                r#"while IFS= read -r line; do
  case "$line" in
    *'"subtype":"initialize"'*) echo '{"type":"control_response","response":{"subtype":"success","request_id":"nola-initialize","response":{}}}' ;;
    *'"type":"user"'*) echo '{"type":"system","subtype":"init","session_id":"original-session"}' ;;
  esac
done
wait"#
            } else {
                r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*|*'"method":"session/resume"'*) echo '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"thread/resume"'*) echo '{"id":2,"result":{"thread":{"id":"original-session"}}}' ;;
    *'"method":"session/prompt"'*|*'"method":"turn/start"'*) echo forbidden > forbidden ;;
  esac
done
wait
"#
            };
            let (config, descriptor) = protocol_fixture(kind.clone(), body);
            let mut observer = TestExternalObserver {
                reject_session: true,
                ..Default::default()
            };
            let result = match kind {
                ExecutionBackendKind::ClaudeCode => run_stream_json_process(
                    &config,
                    "fixture",
                    true,
                    &mut observer,
                    None,
                    &descriptor,
                ),
                ExecutionBackendKind::OpenCode => {
                    run_opencode_acp(&config, "fixture", true, &mut observer, None, &descriptor)
                }
                _ => {
                    run_codex_app_server(&config, "fixture", true, &mut observer, None, &descriptor)
                }
            };
            assert_eq!(result.unwrap_err(), "fixture admission rejected");
            assert!(!config.project_root.join("forbidden").exists());
            assert_protocol_fixture_reaped(&config);
            fs::remove_dir_all(config.project_root).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn opencode_output_commit_rejection_still_sends_native_cancel_before_eof() {
        let body = r#"
trap 'kill "$(cat descendant.pid)" 2>/dev/null; wait' EXIT
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"id":2,"result":{}}' ;;
    *'"method":"session/prompt"'*) echo '{"method":"session/update","params":{"sessionId":"original-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"fictional output"}}}}' ;;
    *'"method":"session/cancel"'*) echo '{"id":4,"result":{"stopReason":"cancelled"}}'; exit 0 ;;
  esac
done
"#;
        let (config, descriptor) = protocol_fixture(ExecutionBackendKind::OpenCode, body);
        let token = RuntimeExecutionControl::default();
        let mut observer = TestExternalObserver {cancel_on_rejected_text:Some(token.clone()),..Default::default()};
        let error = run_opencode_acp(&config,"fictional output",true,&mut observer,Some(token),&descriptor).unwrap_err();
        assert_eq!(error,"external execution cancelled");
        assert!(observer.stop_confirmed);
        let wire=fs::read_to_string(config.project_root.join("requests.jsonl")).unwrap();
        assert!(wire.contains("session/cancel"));
        assert_protocol_fixture_reaped(&config);
        fs::remove_dir_all(config.project_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn opencode_provider_quota_stops_prompt_without_waiting_for_resource_timeout() {
        let body = r#"
trap 'kill "$(cat descendant.pid)" 2>/dev/null; wait' EXIT
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"jsonrpc":"2.0","id":2,"result":{}}' ;;
    *'"method":"session/prompt"'*) printf '%s\n' 'error="AI_APICallError: Free usage limit reached. fictional-secret"' >&2 ;;
  esac
done
"#;
        let (config, descriptor) = protocol_fixture(ExecutionBackendKind::OpenCode, body);
        let mut observer = TestExternalObserver::default();
        let started = Instant::now();
        let message = run_opencode_acp(&config, "fictional quota fixture", true, &mut observer, None, &descriptor).unwrap_err();
        assert!(message.contains("no remaining free quota"), "{message}");
        assert!(!message.contains("fictional-secret"));
        assert!(!message.contains("sampler"));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_protocol_fixture_reaped(&config);
        fs::remove_dir_all(config.project_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn delegated_opencode_permission_keeps_rpc_call_and_input_without_autoapproval() {
        let (config, descriptor) = protocol_fixture(
            ExecutionBackendKind::OpenCode,
            r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"id":2,"result":{}}' ;;
    *'"method":"session/prompt"'*) echo '{"id":"permission-37","method":"session/request_permission","params":{"sessionId":"original-session","toolCall":{"toolCallId":"call-9","kind":"read","rawInput":{"path":"/private/file"},"locations":[{"path":"/private/file"}]},"options":[{"optionId":"yes","kind":"allow_once"},{"optionId":"no","kind":"reject_once"}]}}' ;;
    *'"id":"permission-37"'*) echo unauthorized > forbidden ;;
  esac
done
wait
"#,
        );
        let mut observer = TestExternalObserver::default();
        let error = run_opencode_acp(&config, "fixture", true, &mut observer, None, &descriptor)
            .unwrap_err();
        assert!(error.contains("no decision received"), "{error}");
        assert_eq!(observer.permissions.len(), 1);
        let (id, params) = &observer.permissions[0];
        assert_eq!(id, "permission-37");
        assert_eq!(params.pointer("/toolCall/toolCallId").unwrap(), "call-9");
        assert_eq!(
            params.pointer("/toolCall/rawInput/path").unwrap(),
            "/private/file"
        );
        assert_eq!(
            params.pointer("/toolCall/locations/0/path").unwrap(),
            "/private/file"
        );
        assert!(!config.project_root.join("forbidden").exists());
        assert_protocol_fixture_reaped(&config);
        fs::remove_dir_all(config.project_root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn delegated_permission_observer_rejection_gates_protocol_write_and_reaps_processes() {
        for (kind, reject_claim) in [
            (ExecutionBackendKind::OpenCode, true),
            (ExecutionBackendKind::OpenCode, false),
            (ExecutionBackendKind::Codex, true),
            (ExecutionBackendKind::Codex, false),
        ] {
            let (config, descriptor) = protocol_fixture(
                kind.clone(),
                r#"
while IFS= read -r line; do
  echo "$line" >> requests.jsonl
  case "$line" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{"protocolVersion":1}}' ;;
    *'"method":"session/load"'*) echo '{"id":2,"result":{}}' ;;
    *'"method":"thread/resume"'*) echo '{"id":2,"result":{"thread":{"id":"original-session"}}}' ;;
    *'"method":"session/prompt"'*) echo '{"id":"permission-37","method":"session/request_permission","params":{"sessionId":"original-session","toolCall":{"toolCallId":"call-9","rawInput":{"path":"/private/file"}},"options":[{"optionId":"yes","kind":"allow_once"}]}}' ;;
    *'"method":"turn/start"'*) echo '{"id":10,"result":{"turn":{"id":"fixture-turn"}}}'; echo '{"id":"permission-37","method":"item/commandExecution/requestApproval","params":{"threadId":"original-session","turnId":"fixture-turn","itemId":"call-9","command":"fixture"}}' ;;
  esac
done
wait
"#,
            );
            let mut observer = TestExternalObserver {
                approve_permission: true,
                reject_permission: reject_claim,
                reject_permission_write: true,
                ..Default::default()
            };
            let result = match kind {
                ExecutionBackendKind::OpenCode => {
                    run_opencode_acp(&config, "fixture", true, &mut observer, None, &descriptor)
                }
                _ => {
                    run_codex_app_server(&config, "fixture", true, &mut observer, None, &descriptor)
                }
            };
            if reject_claim {
                assert_eq!(result.unwrap_err(), "fixture claim admission rejected");
                assert!(observer.permission_writes.is_empty());
                let requests =
                    fs::read_to_string(config.project_root.join("requests.jsonl")).unwrap();
                assert!(
                    !requests.contains("permission-37"),
                    "rejected claim wrote a protocol decision: {requests}"
                );
            } else {
                assert_eq!(result.unwrap_err(), "fixture written admission rejected");
                assert_eq!(
                    observer.permission_writes,
                    vec![("permission-37".into(), "call-9".into())]
                );
            }
            assert_protocol_fixture_reaped(&config);
            fs::remove_dir_all(config.project_root).unwrap();
        }
    }

    #[test]
    fn catalog_is_stable_even_when_optional_clis_are_absent() {
        let catalog = discover_execution_backends();
        assert_eq!(catalog.len(), 4);
        assert_eq!(catalog[0].id, "nola");
        assert!(catalog.iter().any(|item| item.id == "codex"));
        assert!(catalog.iter().any(|item| item.id == "claude_code"));
        assert!(catalog.iter().any(|item| item.id == "opencode"));
    }

    #[test]
    fn built_in_backend_resolution_is_local_and_does_not_probe_external_clis() {
        let started = Instant::now();
        let descriptor = discover_execution_backend("nola", false).expect("built-in backend");
        assert_eq!(descriptor, ExecutionBackendDescriptor::built_in_nola());
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "built-in backend resolution unexpectedly performed external discovery: {:?}",
            started.elapsed()
        );
        assert!(discover_execution_backend("unknown", false).is_none());
    }

    #[test]
    fn executable_discovery_includes_official_user_install_locations() {
        let home = crate::platform_paths::user_home_dir().expect("effective execution home");
        let opencode = executable_search_candidates("opencode");
        assert!(opencode.contains(&home.join(".opencode/bin/opencode")));
        let claude = executable_search_candidates("claude");
        assert!(claude.contains(&home.join(".local/bin/claude")));
        let codex = executable_search_candidates("codex");
        assert!(codex.contains(&home.join(".npm-global/bin/codex")));
    }

    #[test]
    fn catalog_machine_contract_serializes_diagnostics_and_capabilities() {
        let encoded = serde_json::to_value(discover_execution_backends()).unwrap();
        let entries = encoded.as_array().unwrap();
        assert_eq!(entries.len(), 4);
        for entry in entries {
            assert_eq!(entry["schema_version"], EXECUTION_BACKEND_SCHEMA_VERSION);
            assert!(entry["id"].is_string());
            assert!(entry["display_name"].is_string());
            assert!(entry["availability"].is_string());
            assert!(entry["capabilities"]["streams_text"].is_boolean());
            assert!(entry["capabilities"]["supports_resume"].is_boolean());
        }
        let opencode = entries
            .iter()
            .find(|entry| entry["id"] == "opencode")
            .unwrap();
        assert_eq!(opencode["transport"], "json_rpc_stdio");
    }

    #[test]
    fn opencode_acp_updates_preserve_stream_and_tool_lifecycle() {
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"agent_message_chunk",
                "content":{"type":"text","text":"hello"}
            })),
            vec![ExternalExecutorEvent::TextDelta {
                text: "hello".into()
            }]
        );
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"agent_thought_chunk",
                "content":{"type":"text","text":"inspect"}
            })),
            vec![ExternalExecutorEvent::ReasoningDelta {
                text: "inspect".into()
            }]
        );
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"tool_call",
                "toolCallId":"call-1",
                "title":"Read file",
                "rawInput":{"path":"/tmp/a.txt"}
            })),
            vec![ExternalExecutorEvent::ToolStarted {
                call_id: "call-1".into(),
                name: "Read file".into(),
                arguments: serde_json::json!({"path":"/tmp/a.txt"}),
            }]
        );
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"usage_update",
                "used":53000,
                "size":200000
            })),
            vec![ExternalExecutorEvent::Usage {
                used_tokens: 53000,
                context_window_tokens: 200000,
            }]
        );
        assert_eq!(
            parse_opencode_acp_update(&serde_json::json!({
                "sessionUpdate":"tool_call_update",
                "toolCallId":"call-1",
                "title":"Read file",
                "status":"completed",
                "rawOutput":"body"
            })),
            vec![ExternalExecutorEvent::ToolFinished {
                call_id: "call-1".into(),
                name: "Read file".into(),
                output: "body".into(),
                failed: false,
            }]
        );
    }

    #[test]
    fn opencode_acp_permission_requires_an_explicit_decision() {
        let params = serde_json::json!({"toolCall":{"kind":"read","rawInput":{"path":"/tmp/file"}},
            "options":[{"optionId":"yes","kind":"allow_once"},{"optionId":"no","kind":"reject_once"}]});
        assert!(opencode_acp_permission_outcome(&params, None).is_err());
        assert_eq!(
            opencode_acp_permission_outcome(&params, Some(true)).unwrap(),
            serde_json::json!({"outcome":"selected","optionId":"yes"})
        );
        assert_eq!(
            opencode_acp_permission_outcome(&params, Some(false)).unwrap(),
            serde_json::json!({"outcome":"selected","optionId":"no"})
        );
        assert!(opencode_acp_permission_outcome(&serde_json::json!({}), Some(true)).is_err());
    }

    #[test]
    fn installed_entries_have_canonical_absolute_paths() {
        for entry in discover_execution_backends().into_iter().skip(1) {
            if entry.availability == ExecutionBackendAvailability::Available {
                let path = PathBuf::from(entry.executable_path.expect("available path"));
                assert!(path.is_absolute());
                assert!(is_executable(&path));
                assert!(entry.version.is_some());
            }
        }
    }

    #[test]
    fn codex_fixture_preserves_ordered_lifecycle() {
        let lines = [
            r#"{"type":"thread.started","thread_id":"thread-1"}"#,
            r#"{"type":"item.completed","item":{"id":"item-0","type":"agent_message","text":"hello"}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":12,"output_tokens":3}}"#,
        ];
        let events = lines
            .iter()
            .flat_map(|line| parse_event_line(ExecutionBackendKind::Codex, line))
            .collect::<Vec<_>>();
        assert_eq!(
            events[0],
            ExternalExecutorEvent::Started {
                session_id: "thread-1".into()
            }
        );
        assert_eq!(
            events[1],
            ExternalExecutorEvent::TextDelta {
                text: "hello".into()
            }
        );
        assert!(matches!(
            events[2],
            ExternalExecutorEvent::Completed {
                input_tokens: Some(12),
                output_tokens: Some(3),
                ..
            }
        ));
    }

    #[test]
    fn codex_agent_message_items_preserve_paragraph_boundaries() {
        let notifications = [
            serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"itemId":"progress-1","delta":"Inspecting"}
            }),
            serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"itemId":"progress-1","delta":" the project."}
            }),
            serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"itemId":"progress-2","delta":"Reviewing changes."}
            }),
            serde_json::json!({
                "method":"item/agentMessage/delta",
                "params":{"itemId":"final","delta":"\n\nSummary:\n- one\n- two"}
            }),
        ];
        let mut boundary = CodexAgentMessageBoundary::default();
        let mut output = String::new();
        for notification in notifications {
            let events = parse_codex_app_server_message(&notification);
            let ExternalExecutorEvent::TextDelta { text } = &events[0] else {
                panic!("expected text delta");
            };
            if let Some(separator) = boundary.separator_before(&notification, text) {
                output.push_str(separator);
            }
            output.push_str(text);
        }
        assert_eq!(
            output,
            "Inspecting the project.\n\nReviewing changes.\n\nSummary:\n- one\n- two"
        );
    }

    #[test]
    fn codex_agent_message_boundary_requires_item_ids_for_new_blocks() {
        let notification = serde_json::json!({
            "method":"item/agentMessage/delta",
            "params":{"delta":"continued"}
        });
        let mut boundary = CodexAgentMessageBoundary::default();
        assert_eq!(boundary.separator_before(&notification, "first"), None);
        assert_eq!(boundary.separator_before(&notification, "continued"), None);

        assert_eq!(boundary.separator_before(&notification, "next block"), None);
        assert_eq!(boundary.separator_before(&notification, " continued"), None);
    }

    #[test]
    fn claude_snapshot_does_not_duplicate_streamed_text() {
        let delta = r#"{"type":"stream_event","session_id":"s1","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}}"#;
        let snapshot = r#"{"type":"assistant","session_id":"s1","message":{"content":[{"type":"text","text":"hello"}]}}"#;
        assert_eq!(
            parse_event_line(ExecutionBackendKind::ClaudeCode, delta),
            vec![ExternalExecutorEvent::TextDelta {
                text: "hello".into()
            }]
        );
        assert!(parse_event_line(ExecutionBackendKind::ClaudeCode, snapshot).is_empty());
    }

    #[test]
    fn opencode_acp_model_selection_is_not_a_cli_flag() {
        let mut descriptor = ExecutionBackendDescriptor::built_in_nola();
        descriptor.kind = ExecutionBackendKind::OpenCode;
        descriptor.executable_path = Some("/fixture/opencode".into());
        let command = command_for(&descriptor, Path::new("/fixture/workspace"),
            Some("original-child"), Some("openai/selected-model"), None).unwrap();
        assert_eq!(command.arguments, ["acp", "--cwd", "/fixture/workspace", "--print-logs", "--log-level", "ERROR"]);
    }

    #[test]
    fn claude_failed_result_preserves_protocol_errors() {
        for (extra, expected) in [
            (serde_json::json!({"errors":["model unavailable", "login required"]}), "model unavailable\nlogin required"),
            (serde_json::json!({"errors":[]}), "error_during_execution"),
        ] {
            let mut result = serde_json::json!({"type":"result", "subtype":"error_during_execution", "is_error":true});
            result.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            assert_eq!(parse_claude_event(&result), vec![ExternalExecutorEvent::Failed {message:expected.into()}]);
        }
    }

    #[test]
    fn claude_snapshots_preserve_tool_lifecycle_without_repeating_text() {
        let started = r#"{"type":"assistant","session_id":"s1","message":{"content":[{"type":"thinking","thinking":"inspect"},{"type":"tool_use","id":"call-1","name":"Read","input":{"file_path":"/tmp/input.txt"}},{"type":"text","text":"duplicate"}]}}"#;
        assert_eq!(
            parse_event_line(ExecutionBackendKind::ClaudeCode, started),
            vec![ExternalExecutorEvent::ToolStarted {
                call_id: "call-1".into(),
                name: "Read".into(),
                arguments: serde_json::json!({"file_path":"/tmp/input.txt"}),
            }]
        );
        let finished = r#"{"type":"user","session_id":"s1","message":{"content":[{"type":"tool_result","tool_use_id":"call-1","content":"file body"}]}}"#;
        assert_eq!(
            parse_event_line(ExecutionBackendKind::ClaudeCode, finished),
            vec![ExternalExecutorEvent::ToolFinished {
                call_id: "call-1".into(),
                name: "Claude Code tool".into(),
                output: "file body".into(),
                failed: false,
            }]
        );
    }

    #[test]
    fn native_accounting_keeps_cumulative_scope_and_unknown_counters() {
        use nola_domain::delegated_invocation::ExternalUsageBasis;
        let frame=serde_json::json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-1",
            "tokenUsage":{"total":{"inputTokens":1200,"cachedInputTokens":800,"outputTokens":50},"last":{"inputTokens":200}}}});
        let first=external_accounting_snapshot("codex",&frame).unwrap();
        assert_eq!(first.basis,ExternalUsageBasis::SessionCumulative);
        assert_eq!(first.scope_id,"thread-1"); assert_eq!(first.input_tokens,Some(1200));
        assert_eq!(first.cache_read_tokens,Some(800)); assert_eq!(first.cache_write_tokens,None);
        assert_eq!(external_accounting_snapshot("codex",&frame),Some(first));
        let claude=external_accounting_snapshot("claude_code",&serde_json::json!({"type":"result","session_id":"session",
            "usage":{"input_tokens":10,"cache_read_input_tokens":90,"cache_creation_input_tokens":20,"output_tokens":5},
            "modelUsage":{"actual-model":{}}})).unwrap();
        assert_eq!(claude.input_tokens,Some(120)); assert_eq!(claude.output_tokens,Some(5));
        assert_eq!(claude.model_id.as_deref(),Some("actual-model")); assert_eq!(claude.provider_id,None);
        assert!(external_accounting_snapshot("opencode",&serde_json::json!({"sessionUpdate":"usage_update","used":100,"size":10000})).is_none());
        let opencode=external_accounting_snapshot("opencode",&serde_json::json!({"result":{"usage":{"inputTokens":3,"outputTokens":8,"cachedReadTokens":90,"cachedWriteTokens":7}}})).unwrap();
        assert_eq!(opencode.basis,ExternalUsageBasis::LastModelResponse);
        assert_eq!(opencode.input_tokens,Some(100)); assert_eq!(opencode.cache_read_tokens,Some(90));
    }

    #[test]
    fn claude_result_projects_cache_aware_authoritative_context_usage() {
        let result = serde_json::json!({
            "type":"result",
            "session_id":"s1",
            "is_error":false,
            "usage":{
                "input_tokens":120,
                "cache_read_input_tokens":18000,
                "cache_creation_input_tokens":880,
                "output_tokens":240
            },
            "modelUsage":{
                "claude-opus-4-7":{
                    "inputTokens":120,
                    "cacheReadInputTokens":18000,
                    "cacheCreationInputTokens":880,
                    "outputTokens":240,
                    "contextWindow":200000
                }
            }
        });
        assert_eq!(
            parse_claude_event(&result),
            vec![
                ExternalExecutorEvent::Usage {
                    used_tokens: 19000,
                    context_window_tokens: 200000,
                },
                ExternalExecutorEvent::Completed {
                    session_id: Some("s1".into()),
                    input_tokens: Some(19000),
                    output_tokens: Some(240),
                },
            ]
        );
    }

    #[test]
    fn command_builders_keep_global_codex_flags_before_exec() {
        let descriptor = discover_execution_backends()
            .into_iter()
            .find(|item| item.id == "codex")
            .unwrap();
        if descriptor.availability != ExecutionBackendAvailability::Available {
            return;
        }
        let command = command_for(
            &descriptor,
            Path::new("/tmp/project"),
            None,
            Some("gpt-test"),
            Some("test_provider"),
        )
        .unwrap();
        assert!(command
            .arguments
            .windows(2)
            .any(|pair| pair == ["--model", "gpt-test"]));
        assert!(command
            .arguments
            .windows(2)
            .any(|pair| pair == ["-c", "model_provider=test_provider"]));
        assert!(command
            .arguments
            .windows(2)
            .any(|pair| pair == ["--sandbox", "workspace-write"]));
        let exec = command
            .arguments
            .iter()
            .position(|arg| arg == "exec")
            .unwrap();
        let sandbox = command
            .arguments
            .iter()
            .position(|arg| arg == "--sandbox")
            .unwrap();
        assert!(sandbox < exec);
        assert_eq!(command.arguments.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn codex_resume_keeps_resume_only_flags_after_subcommand() {
        let descriptor = discover_execution_backends()
            .into_iter()
            .find(|item| item.id == "codex")
            .unwrap();
        if descriptor.availability != ExecutionBackendAvailability::Available {
            return;
        }
        let command = command_for(
            &descriptor,
            Path::new("/tmp/project"),
            Some("thread-1"),
            None,
            None,
        )
        .unwrap();
        let resume = command
            .arguments
            .iter()
            .position(|arg| arg == "resume")
            .unwrap();
        assert_eq!(
            &command.arguments[resume..],
            &["resume", "--json", "--skip-git-repo-check", "thread-1", "-"]
        );
    }

    #[test]
    fn claude_permission_bridge_requires_owned_session_call_and_explicit_decision() {
        let mut descriptor = ExecutionBackendDescriptor::built_in_nola();
        descriptor.kind = ExecutionBackendKind::ClaudeCode;
        descriptor.executable_path = Some("/fictional/claude".into());
        let command = command_for(&descriptor, Path::new("/fictional"), None, None, None).unwrap();
        assert!(command.arguments.windows(2).any(|v| v == ["--permission-mode", "default"]));
        assert!(command.arguments.windows(2).any(|v| v == ["--permission-prompt-tool", "stdio"]));
        assert!(!command.arguments.iter().any(|v| v == "dontAsk" || v.contains("bypass")));
        let mut control = ClaudeControl::default();
        let tools = HashMap::from([("call-1".into(), ("Write".into(), serde_json::json!({"file_path":"/isolated/result.txt","content":"fictional"}))) ]);
        let frame = serde_json::json!({"type":"control_request", "request_id":"permission-1", "request":{
            "subtype":"can_use_tool", "tool_use_id":"call-1", "tool_name":"Write", "input":tools["call-1"].1}});
        assert_eq!(control.initialize()["request"]["subtype"], "initialize");
        assert!(control.permission(&frame, Some("owned-session"), &tools).is_err());
        let initialized = serde_json::json!({"type":"control_response", "response":{"request_id":"nola-initialize","subtype":"success","response":{}}});
        control.accept_initialize(&initialized).unwrap();
        assert!(control.accept_initialize(&initialized).is_err());
        assert!(control.permission(&frame, None, &tools).is_err());
        let mut foreign = frame.clone(); foreign["session_id"] = "foreign-session".into();
        assert!(control.permission(&foreign, Some("owned-session"), &tools).is_err());
        let mut changed = frame.clone(); changed["request"]["input"]["content"] = "changed after admission".into();
        assert!(control.permission(&changed, Some("owned-session"), &tools).is_err());
        let permission = control.permission(&frame, Some("owned-session"), &tools).unwrap();
        assert!(permission.response(None).is_err());
        let denied = permission.response(Some(false)).unwrap();
        assert_eq!(denied["response"]["request_id"], "permission-1");
        assert_eq!(denied["response"]["response"]["behavior"], "deny");
        let allowed = permission.response(Some(true)).unwrap();
        assert_eq!(allowed["response"]["response"]["updatedInput"], tools["call-1"].1);
        assert!(allowed["response"]["response"].get("updatedPermissions").is_none());
        assert!(control.permission(&frame, Some("owned-session"), &tools).is_err());
        let mut early = ClaudeControl::default(); early.accept_initialize(&initialized).unwrap();
        early.permission(&frame, Some("owned-session"), &HashMap::new()).unwrap();
        early.validate_tool("call-1", "Write", &tools["call-1"].1).unwrap();
        assert!(early.validate_tool("call-1", "Write", &serde_json::json!({"content":"changed"})).is_err());
        control.observe_tasks(&serde_json::json!({"type":"system","subtype":"task_started","task_id":"child"}));
        assert!(!control.tasks.is_empty(), "Keep permission input open for a live native child");
        control.observe_tasks(&serde_json::json!({"type":"system","subtype":"task_notification","task_id":"child"}));
        assert!(control.tasks.is_empty());
    }

    #[test]
    fn claude_uses_native_stream_input_for_new_and_resumed_sessions() {
        let descriptor = discover_execution_backends()
            .into_iter()
            .find(|item| item.id == "claude_code")
            .unwrap();
        if descriptor.availability != ExecutionBackendAvailability::Available {
            return;
        }
        let fresh = command_for(
            &descriptor,
            Path::new("/tmp/project"),
            None,
            Some("sonnet"),
            None,
        )
        .unwrap();
        assert!(fresh.prompt_via_stdin);
        assert!(fresh
            .arguments
            .windows(2)
            .any(|pair| pair == ["--input-format", "stream-json"]));
        assert!(!fresh
            .arguments
            .iter()
            .any(|argument| argument == "--resume"));
        assert!(fresh
            .arguments
            .windows(2)
            .any(|pair| pair == ["--model", "sonnet"]));

        let resumed = command_for(
            &descriptor,
            Path::new("/tmp/project"),
            Some("session-1"),
            Some("sonnet"),
            None,
        )
        .unwrap();
        assert!(resumed.prompt_via_stdin);
        assert!(resumed
            .arguments
            .windows(2)
            .any(|pair| pair == ["--resume", "session-1"]));
    }

    #[test]
    fn external_image_content_is_native_for_claude_and_opencode() {
        let root =
            std::env::temp_dir().join(format!("nola-external-image-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("sample.png");
        std::fs::write(&image, b"not-empty-image-fixture").unwrap();
        let attachments = vec![crate::external_execution_settings::HostAttachment {
            path: image.clone(),
            name: "sample.png".to_string(),
            mime_type: "image/png".to_string(),
            byte_count: std::fs::metadata(&image).unwrap().len(),
        }];

        let content = external_image_content_blocks(
            &attachments,
            "describe it".to_string(),
            ExternalImageBlockStyle::Claude,
        )
        .unwrap();
        let claude = serde_json::json!({
            "type":"user",
            "message":{"role":"user", "content":content}
        })
        .to_string();
        let claude: Value = serde_json::from_str(claude.trim()).unwrap();
        assert_eq!(claude["message"]["content"][1]["type"], "image");
        assert_eq!(
            claude["message"]["content"][1]["source"]["media_type"],
            "image/png"
        );

        let acp = external_image_content_blocks(
            &attachments,
            "describe it".to_string(),
            ExternalImageBlockStyle::Acp,
        )
        .unwrap();
        assert_eq!(acp[1]["type"], "image");
        assert_eq!(acp[1]["mimeType"], "image/png");
        assert!(!acp[1]["data"].as_str().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_native_image_at_thirty_megabyte_product_limit_is_preserved() {
        let root = std::env::temp_dir().join(format!(
            "nola-external-large-image-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("large.png");
        let file = std::fs::File::create(&image).unwrap();
        file.set_len(30 * 1024 * 1024).unwrap();
        let attachments = vec![crate::external_execution_settings::HostAttachment {
            path: image.clone(),
            name: "large.png".to_string(),
            mime_type: "image/png".to_string(),
            byte_count: 30 * 1024 * 1024,
        }];

        let content = external_image_content_blocks(
            &attachments,
            "inspect".to_string(),
            ExternalImageBlockStyle::Claude,
        )
        .unwrap();
        assert!(content[1]["source"]["data"].as_str().unwrap().len() > 40_000_000);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opencode_current_json_contract_preserves_session_tools_and_terminal_event() {
        let lines = [
            r#"{"type":"step_start","timestamp":1,"sessionID":"ses-1","part":{"type":"step-start"}}"#,
            r#"{"type":"tool_use","timestamp":2,"sessionID":"ses-1","part":{"id":"part-1","callID":"call-1","tool":"read","state":{"status":"completed","input":{"filePath":"README.md"},"output":"contents"}}}"#,
            r#"{"type":"text","timestamp":3,"sessionID":"ses-1","part":{"type":"text","text":"answer"}}"#,
            r#"{"type":"step_finish","timestamp":4,"sessionID":"ses-1","part":{"type":"step-finish","tokens":{"input":12,"output":4}}}"#,
        ];
        let events = lines
            .iter()
            .flat_map(|line| parse_event_line(ExecutionBackendKind::OpenCode, line))
            .collect::<Vec<_>>();
        assert_eq!(
            events[0],
            ExternalExecutorEvent::Started {
                session_id: "ses-1".into()
            }
        );
        assert!(
            matches!(&events[1], ExternalExecutorEvent::ToolStarted { call_id, name, .. } if call_id == "call-1" && name == "read")
        );
        assert!(
            matches!(&events[2], ExternalExecutorEvent::ToolFinished { call_id, output, failed: false, .. } if call_id == "call-1" && output == "contents")
        );
        assert_eq!(
            events[3],
            ExternalExecutorEvent::TextDelta {
                text: "answer".into()
            }
        );
        assert!(
            matches!(events[4], ExternalExecutorEvent::Completed { session_id: Some(ref id), input_tokens: Some(12), output_tokens: Some(4) } if id == "ses-1")
        );
    }

    #[test]
    fn opencode_nested_api_error_remains_actionable() {
        let line = r#"{"type":"error","sessionID":"ses-2","error":{"name":"APIError","data":{"message":"No endpoints found that support tool use","statusCode":404}}}"#;
        assert_eq!(
            parse_event_line(ExecutionBackendKind::OpenCode, line),
            vec![ExternalExecutorEvent::Failed {
                message: "No endpoints found that support tool use".into(),
            }]
        );
    }

    #[test]
    fn external_prompt_uses_typed_nola_context_without_replaying_chat_history() {
        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.host_context_components = vec![nola_core::context_engine::ContextComponent {
            kind: nola_core::context_engine::ContextComponentKind::Document,
            id: "document-1".to_string(),
            content: Value::String("trusted reference".to_string()),
            source: None,
        }];
        let prompt = compose_external_prompt(&config, "current request");
        assert!(prompt.contains("<nola_turn_context>"));
        assert!(prompt.contains("<nola_execution_contract>"));
        assert!(prompt.contains("<nola_action_scope>"));
        assert!(prompt.contains("code in the conversation by default"));
        assert!(prompt.contains("user-facing assistant"));
        assert!(prompt.contains(&config.project_root.display().to_string()));
        assert!(prompt.contains("trusted reference"));
        assert!(prompt.contains("<user_request>\ncurrent request"));
        assert!(!prompt.contains("previous assistant answer"));
    }

    #[test]
    fn external_prompt_keeps_identity_and_workspace_without_optional_context() {
        let config = Config::for_test().unwrap().external_execution_settings();
        let prompt = compose_external_prompt(&config, "who are you?");
        assert!(prompt.contains("<nola_execution_contract>"));
        assert!(prompt.contains("inside Nola"));
        assert!(prompt.contains(&config.project_root.display().to_string()));
        assert!(prompt.contains("<user_request>\nwho are you?"));
        assert!(!prompt.contains("<nola_turn_context>"));
    }

    #[test]
    fn resumed_external_prompt_does_not_repeat_the_stable_execution_contract() {
        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.external_execution_session_id = Some("existing-session".to_string());
        let prompt = compose_external_prompt(&config, "continue");
        assert!(!prompt.contains("<nola_execution_contract>"));
        assert!(prompt.contains("<nola_action_scope>"));
        assert!(prompt.contains("<user_request>\ncontinue"));
    }

    #[test]
    fn codex_model_list_uses_protocol_display_names_and_default() {
        let response = serde_json::json!({
            "id": 2,
            "result": {
                "data": [
                    {"id":"catalog-a","model":"gpt-a","displayName":"GPT A","hidden":false,"isDefault":true},
                    {"id":"catalog-hidden","model":"gpt-hidden","displayName":"Hidden","hidden":true,"isDefault":false},
                    {"id":"catalog-b","model":"gpt-b","displayName":"GPT B","hidden":false,"isDefault":false}
                ]
            }
        });
        let (models, default_model) = parse_codex_model_list_response(&response).unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gpt-a", "gpt-b"]
        );
        assert_eq!(
            models
                .iter()
                .map(|model| model.display_name.as_str())
                .collect::<Vec<_>>(),
            vec!["GPT A", "GPT B"]
        );
        assert_eq!(default_model.as_deref(), Some("gpt-a"));
    }

    #[cfg(unix)]
    #[test]
    fn external_executor_termination_kills_descendants_and_leaves_no_orphan() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "nola-external-process-tree-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create process-tree fixture directory");
        let descendant_pid_path = directory.join("descendant.pid");
        let script = format!(
            "sleep 30 & echo $! > '{}'; wait",
            descendant_pid_path.display()
        );
        let mut command = external_command(Path::new("/bin/sh"));
        let mut child = command
            .args(["-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn external process tree");

        let deadline = Instant::now() + Duration::from_secs(2);
        while !descendant_pid_path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let descendant_pid = std::fs::read_to_string(&descendant_pid_path)
            .expect("external descendant pid")
            .trim()
            .parse::<i32>()
            .expect("numeric descendant pid");

        terminate_external_child(&mut child).expect("terminate external process tree");
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(descendant_pid, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            unsafe { libc::kill(descendant_pid, 0) },
            0,
            "external executor descendant remained alive after cancellation"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(unix)]
    #[test]
    fn dropping_external_executor_after_protocol_error_kills_its_process_tree() {
        let mut command = external_command(Path::new("/bin/sh"));
        let child = ManagedExternalChild::new(
            command
                .args(["-c", "sleep 30 & wait"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn guarded external process tree"),
        );
        let process_group = i32::try_from(child.id()).expect("external process group id");
        std::thread::sleep(Duration::from_millis(50));
        drop(child);

        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { libc::kill(-process_group, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_ne!(
            unsafe { libc::kill(-process_group, 0) },
            0,
            "protocol-error cleanup left the external process group alive"
        );
    }

    #[test]
    fn oversized_external_protocol_line_is_rejected_without_unbounded_buffering() {
        let output_bytes = Arc::new(AtomicU64::new(0));
        let receiver = bounded_external_line_receiver(
            std::io::Cursor::new(vec![b'x'; 1_025]),
            1_024,
            Arc::clone(&output_bytes),
        );
        let error = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("bounded protocol reader result")
            .expect_err("oversized protocol line must fail");
        assert!(
            error.contains("protocol-line budget of 1024 bytes"),
            "{error}"
        );
        assert_eq!(output_bytes.load(Ordering::Relaxed), 1_025);
    }

    #[cfg(unix)]
    #[test]
    fn exited_external_child_preserves_real_status_without_inventing_a_resource_sample() {
        let config = Config::for_test().unwrap().external_execution_settings();
        let mut monitor = ExternalExecutionBudgetMonitor::new(
            ExternalExecutionBudget::from_config(&config), Arc::new(AtomicU64::new(0)));
        let mut child = std::process::Command::new("/bin/sh").args(["-c", "exit 23"]).spawn().unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(23));
        monitor.check(&mut child).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(23));
    }

    #[cfg(unix)]
    #[test]
    fn external_process_tree_exceeding_process_budget_is_detected_and_reaped() {
        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.budget.max_processes = Some(1);
        let budget = ExternalExecutionBudget::from_config(&config);
        let output_bytes = Arc::new(AtomicU64::new(0));
        let mut monitor = ExternalExecutionBudgetMonitor::new(budget, output_bytes);
        let mut command = external_command(Path::new("/bin/sh"));
        let child = command
            .args(["-c", "sleep 30 & wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn budgeted external process tree");
        let mut child = ManagedExternalChild::new(child);
        let process_group = i32::try_from(child.id()).expect("external process group id");

        let deadline = Instant::now() + Duration::from_secs(2);
        let error = loop {
            match monitor.check(&mut child) {
                Ok(()) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(110));
                }
                Ok(()) => panic!("external process-count budget was not enforced"),
                Err(error) => break error,
            }
        };
        assert!(error.contains("process-count budget of 1"), "{error}");
        drop(child);
        assert_ne!(
            unsafe { libc::kill(-process_group, 0) },
            0,
            "budget failure left the external process group alive"
        );
    }

    #[cfg(unix)]
    #[test]
    fn real_stream_json_executor_budget_failure_propagates_and_reaps_process_tree() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "nola-external-budget-e2e-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create external budget fixture");
        let executable = directory.join("fake-claude");
        let pid_path = directory.join("process-group.pid");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\necho $$ > '{}'\nsleep 30 &\nIFS= read -r initialize\necho '{{\"type\":\"control_response\",\"response\":{{\"subtype\":\"success\",\"request_id\":\"nola-initialize\",\"response\":{{}}}}}}'\nIFS= read -r prompt\necho '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"budget-session\"}}'\nwait\n",
                pid_path.display()
            ),
        )
        .expect("write fake Claude executable");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fake Claude metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("make fake Claude executable");

        let mut config = Config::for_test().unwrap().external_execution_settings();
        config.project_root = directory.clone();
        config.budget.max_processes = Some(1);
        let descriptor = ExecutionBackendDescriptor {
            schema_version: EXECUTION_BACKEND_SCHEMA_VERSION,
            id: "claude_code".to_string(),
            kind: ExecutionBackendKind::ClaudeCode,
            display_name: "Fake Claude Code".to_string(),
            availability: ExecutionBackendAvailability::Available,
            transport: ExecutionBackendTransport::StreamJsonProcess,
            executable_path: Some(executable.to_string_lossy().into_owned()),
            version: Some("test".to_string()),
            capabilities: ExecutionBackendCapabilities {
                streams_text: true,
                streams_reasoning: true,
                streams_tools: true,
                supports_resume: true,
                supports_cancel: true,
                supports_permission_bridge: false,
            },
            diagnostic: None,
            models: Vec::new(),
            default_model_id: None,
        };
        let mut observer = TestExternalObserver::default();
        let error = run_stream_json_process(
            &config,
            "budget test",
            true,
            &mut observer,
            None,
            &descriptor,
        )
        .expect_err("external process-count budget must fail the real protocol run");
        assert!(error.contains("process-count budget of 1"), "{error}");

        let process_group = std::fs::read_to_string(&pid_path)
            .expect("fake Claude process group")
            .trim()
            .parse::<i32>()
            .expect("numeric process group");
        assert_ne!(
            unsafe { libc::kill(-process_group, 0) },
            0,
            "real protocol budget failure left the process tree alive"
        );
        let _ = std::fs::remove_dir_all(directory);
    }
}
