// SPDX-License-Identifier: Apache-2.0
// Real delegated process -> Host checked delivery -> in-process Application.
// Checks durable delegated Invocation identity and rollback through the real Host.
// This in-process runtime path uses ordinary Application commits, not receipt admission.
#[cfg(all(test, unix))]
mod delegated_commit_application_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const REJECTED_TEXT: &str = "REJECTED_DELEGATED_OUTPUT";

    struct Fixture {
        root: std::path::PathBuf,
        session: String,
        environment: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }
    impl Fixture {
        fn pids(&self) -> Vec<i32> {
            std::fs::read_to_string(self.root.join("business-pids"))
                .unwrap_or_default()
                .split_whitespace()
                .map(|pid| pid.parse().unwrap())
                .collect()
        }
        fn kill_owned_processes(&self) {
            for pid in self.pids() {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.kill_owned_processes();
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

    fn scenario(rejected_event: &str) {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-delegated-commit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let fixture = Fixture {
            root: root.clone(),
            session: format!("delegated-commit-{}", uuid::Uuid::new_v4()),
            environment: [
                "NOLA_HOME",
                "NOLA_DB_PATH",
                "PATH",
                "NOLA_DELEGATED_TEST_ROOT",
                "NOLA_DELEGATED_TERMINAL_EXIT",
            ]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect(),
        };
        std::env::set_var("NOLA_HOME", &root);
        let database = root.join("canonical.sqlite");
        std::env::set_var("NOLA_DB_PATH", &database);
        std::env::set_var("NOLA_DELEGATED_TEST_ROOT", &root);
        if matches!(rejected_event, "completed" | "success") {
            std::env::set_var("NOLA_DELEGATED_TERMINAL_EXIT", "1");
        } else {
            std::env::remove_var("NOLA_DELEGATED_TERMINAL_EXIT");
        }
        let old_path = std::env::var_os("PATH").unwrap_or_default();
        let mut path = vec![root.clone()];
        path.extend(std::env::split_paths(&old_path));
        std::env::set_var("PATH", std::env::join_paths(path).unwrap());
        let executable = root.join("claude");
        std::fs::write(&executable,r#"#!/usr/bin/env python3
import json, os, pathlib, sys
root=pathlib.Path(os.environ['NOLA_DELEGATED_TEST_ROOT'])
if '--version' in sys.argv or (len(sys.argv)>1 and sys.argv[1]=='auth'):
    with (root/'probes').open('a') as f: f.write('probe\n')
    print('fake-claude 1.0')
    sys.exit(0)
# Consume the real Host prompt before publishing events; production writes the
# stream-json input to stdin before it starts reading stdout.
sys.stdin.read()
read_fd, write_fd=os.pipe()
child=os.fork()
if child==0:
    os.read(read_fd,1)
    os._exit(0)
(root/'business-pids').write_text(str(os.getpid())+' '+str(child))
print(json.dumps({'type':'system','subtype':'init','session_id':'delegated-commit-backend'}),flush=True)
print(json.dumps({'type':'stream_event','session_id':'delegated-commit-backend','event':{'type':'content_block_delta','delta':{'type':'text_delta','text':'REJECTED_DELEGATED_OUTPUT'}}}),flush=True)
print(json.dumps({'type':'result','session_id':'delegated-commit-backend','is_error':False,'result':'done','usage':{'input_tokens':3,'output_tokens':1}}),flush=True)
if os.environ.get('NOLA_DELEGATED_TERMINAL_EXIT') == '1':
    os.write(write_fd,b'x')
    os.waitpid(child,0)
    sys.exit(0)
# Deliberately remain live on an explicit pipe, so rejection must terminate both
# processes. No timing-based side-effect claim is made by this fixture.
os.read(read_fd,1)
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = crate::config::Config::from_args(vec![
            "nola".into(),
            "--session".into(),
            fixture.session.clone(),
            "--project".into(),
            root.to_string_lossy().into_owned(),
            "--endpoint".into(),
            "http://127.0.0.1:1/v1".into(),
            "--api-key".into(),
            "fixture".into(),
            "--model".into(),
            "fixture-model".into(),
        ])
        .and_then(|config| config.host_invocation_request())
        .unwrap();
        // Exercise the independently selected child profile, rather than
        // selecting Claude for the parent conversation itself.
        assert_eq!(config.execution.execution_backend, "nola");
        let registry = crate::agent_registry::AgentRegistry::from_profiles(&Default::default());
        let store = crate::store::NolaStore::open_default().unwrap();
        let snapshot = crate::delegated_invocation::DelegatedInvocation::capture(
            &config, &fixture.session, registry.get("claude_code").unwrap(), &store,
        ).unwrap();
        let (_, child) = snapshot.restore(&config, &fixture.session, &store).unwrap();
        config = child;
        assert_eq!(config.execution.execution_backend, "claude_code");
        assert!(config.model_settings.api_key.is_empty());
        config.session.save_session = true;
        config.session.supports_reviewed_text_parts = false;
        config.tool_settings.tools_enabled = false;
        config.tool_settings.skill_dirs = Some(Vec::new());
        config.tool_settings.allowed_dirs = vec![root.clone()];
        config.runtime.compaction.context_window_tokens = Some(32768);
        let host = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let db = rusqlite::Connection::open(&database).unwrap();
        let predicate = if rejected_event == "success" {
            "0"
        } else if rejected_event == "execution_started" {
            "json_extract(NEW.payload_json,'$.payload.type')='execution_started'"
        } else if rejected_event == "session_started" {
            "json_extract(NEW.payload_json,'$.payload.type')='delegated_agent_event' AND json_extract(NEW.payload_json,'$.payload.event.type')='session_started'"
        } else if rejected_event == "completed" {
            "json_extract(NEW.payload_json,'$.payload.type')='delegated_agent_event' AND json_extract(NEW.payload_json,'$.payload.event.type')='completed'"
        } else {
            "json_extract(NEW.payload_json,'$.payload.type')='delegated_agent_event' AND json_extract(NEW.payload_json,'$.payload.event.type')='text_delta'"
        };
        db.execute_batch(&format!("CREATE TRIGGER reject_delegated_commit BEFORE INSERT ON host_event_inbox WHEN {predicate} BEGIN SELECT RAISE(ABORT,'delegated commit admission fault'); END")).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        // This debug-only fixture nests Host, Store fault injection and reducer
        // frames. Match the explicit local budget used by permission recovery
        // fixtures; production thread policy is not changed by this test.
        let execution = std::thread::Builder::new()
            .name("delegated-commit-fixture".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
            let mut emitter =
                Emitter::with_event_sink(move |value| sink.lock().unwrap().push(value));
            let result = host.lock().unwrap().run_turn_items(
                &config,
                host_user_input_items(&config, "Return a short answer"),
                nola_core::context_engine::ContextPlatform::Unknown,
                Vec::new(),
                RuntimeExecutionControl::default(),
                &mut emitter,
            );
            let _ = tx.send(result);
        }).expect("spawn delegated commit fixture");
        let result = match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(result) => result,
            Err(error) => {
                fixture.kill_owned_processes();
                let _ = rx.recv_timeout(Duration::from_secs(5));
                panic!("Host failed to return after rejected delegated commit: {error}");
            }
        };
        execution.join().unwrap();
        if rejected_event == "success" {
            result.expect("independent Claude profile must reach the real stub driver and complete");
            assert_eq!(db.query_row::<String, _, _>("SELECT state FROM delegated_invocations", [], |r| r.get(0)).unwrap(), "succeeded");
            assert_eq!(db.query_row::<i64, _, _>("SELECT count(*) FROM run_events WHERE event_kind='turn_completed'", [], |r| r.get(0)).unwrap(), 1);
            let checkpoint: String = db.query_row("SELECT checkpoint_json FROM core_checkpoint_bodies", [], |r| r.get(0)).unwrap();
            assert!(checkpoint.contains(REJECTED_TEXT), "accepted streamed output was lost");
            assert_eq!(fixture.pids().len(), 2, "the external driver must launch its process and descendant");
            for pid in fixture.pids() { assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "completed stub left process alive"); }
            return;
        }
        let failure: Option<String> = db.query_row("SELECT group_concat(substr(value,1,512)) FROM core_checkpoint_bodies,json_tree(checkpoint_json) WHERE key IN ('failure','message') AND type='text'", [], |row| row.get(0)).unwrap();
        let error = match result { Err(error) => error, Ok(outcome) => panic!("Expected rejected commit, got {outcome:?}; checkpoint failure={failure:?}; pids={:?}",fixture.pids()) };
        assert!(
            error.contains("delegated commit admission fault"),
            "Unexpected failure before Application gate: {error}"
        );
        if rejected_event == "execution_started" {
            assert!(
                fixture.pids().is_empty(),
                "A rejected Accepted transaction cannot launch the business process"
            );
            assert!(!root.join("business-pids").exists());
        } else {
            let pids = fixture.pids();
            assert_eq!(
                pids.len(),
                2,
                "Fixture must have launched a parent and descendant"
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while pids.iter().any(|pid| unsafe { libc::kill(*pid, 0) } == 0)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            for pid in pids {
                assert_eq!(
                    unsafe { libc::kill(pid, 0) },
                    -1,
                    "Rejected output left process {pid} alive"
                );
            }
        }
        assert!(
            rejected_event == "execution_started" || root.join("probes").exists(),
            "Probe execution must be distinguished from business execution"
        );
        let rejected_count: i64 = db
            .query_row(
                &format!("SELECT count(*) FROM host_event_inbox AS NEW WHERE {predicate}"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            rejected_count, 0,
            "Rejected Host response cannot become a canonical inbox fact"
        );
        let records: Vec<Value> = db.prepare("SELECT record_json FROM delegated_invocations")
            .unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
            .map(|row| serde_json::from_str(&row.unwrap()).unwrap()).collect();
        assert_eq!(records.len(), 1, "One logical external invocation must own the delivery");
        let invocation = &records[0];
        let invocation_id = invocation["id"].as_str().unwrap();
        uuid::Uuid::parse_str(invocation_id).expect("Application creates a distinct invocation UUID");
        assert!(invocation["completed_at_ms"].is_null());
        assert!(invocation["failure"].is_null());
        if rejected_event == "execution_started" {
            assert_eq!(invocation["state"], "prepared");
            assert_eq!(invocation["revision"], 1);
            assert!(invocation["started_at_ms"].is_null(), "Rejected admission cannot start the ledger");
            assert!(invocation["backend_session_id"].is_null());
        } else {
            assert_eq!(invocation["state"], "running");
            assert!(invocation["started_at_ms"].as_i64().is_some());
            if rejected_event == "session_started" {
                assert!(invocation["backend_session_id"].is_null(), "Rejected backend identity cannot enter ledger");
                assert_eq!(invocation["revision"], 2);
            } else {
                assert_eq!(invocation["backend_session_id"], "delegated-commit-backend");
                assert_eq!(invocation["revision"], 3);
            }
        }
        assert_eq!(db.query_row::<i64, _, _>("SELECT count(*) FROM run_attempts", [], |r| r.get(0)).unwrap(), 0,
            "Opaque delegated sampling must not create invented model Attempts");
        let bindings: Vec<String> = db.prepare("SELECT request_id FROM delegated_invocation_deliveries WHERE invocation_id=?1")
            .unwrap().query_map([invocation_id], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
        assert!(!bindings.is_empty(), "Prepared invocation must bind its committed Host delivery");
        assert!(bindings.iter().all(|request_id| request_id != invocation_id));
        let accepted_identities: Vec<Option<String>> = db.prepare("SELECT json_extract(payload_json,'$.delegated_invocation_id') FROM host_event_inbox WHERE json_extract(payload_json,'$.payload.type') IN ('accepted','delegated_agent_event')")
            .unwrap().query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect();
        assert!(accepted_identities.iter().all(|id| id.as_deref() == Some(invocation_id)),
            "Every committed delegated response must retain the independent invocation owner");
        let checkpoint: String = db
            .query_row("SELECT checkpoint_json FROM core_checkpoint_bodies", [], |row| {
                row.get(0)
            })
            .unwrap();
        if rejected_event != "completed" {
            assert!(
                !checkpoint.contains(REJECTED_TEXT),
                "Rejected content entered the canonical checkpoint"
            );
        }
        let mut query = db
            .prepare(
                "SELECT COALESCE(content_text,''),COALESCE(content_json,'') FROM message_parts",
            )
            .unwrap();
        for row in query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
        {
            let (text, json) = row.unwrap();
            if rejected_event != "completed" {
                assert!(!text.contains(REJECTED_TEXT) && !json.contains(REJECTED_TEXT));
            }
        }
        let delivery_state: String = db
            .query_row(
                "SELECT state FROM effect_delivery_attempts",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            delivery_state,
            if rejected_event == "execution_started" {
                "leased"
            } else {
                "execution_started"
            },
            "Application must retain the last committed physical-delivery fact"
        );
        if rejected_event == "completed" {
            invalidate_core_agent_session(&fixture.session).unwrap();
            let mut application = crate::application_port::CliApplicationPort::persistent(&database)
                .unwrap();
            let _: usize = application
                .execute(nola_application::ApplicationCommand::RecoverOrphanedRuns {
                    conversation_id: Some(fixture.session.clone()),
                    reason: "test_process_restarted".into(),
                    interrupted_at: "after_terminal_commit_rejection".into(),
                    interrupted_at_ms: chrono::Local::now().timestamp_millis(),
                })
                .unwrap();
            let recovered_state: String = db
                .query_row(
                    "SELECT state FROM effect_delivery_attempts",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(recovered_state, "outcome_unknown");
        }
        assert_eq!(
            db.query_row::<i64, _, _>(
                "SELECT count(*) FROM run_events WHERE event_kind='turn_completed'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
            0,
            "A transport admission failure is not successful completion"
        );
        assert!(!events
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.pointer("/trace/kind").and_then(Value::as_str)
                == Some("turn.completed")));
    }

    #[test]
    fn independent_profile_reaches_external_process_and_commits_its_result() {
        scenario("success");
    }

    #[test]
    fn rejected_initial_execution_started_commit_never_spawns_business_process() {
        scenario("execution_started");
    }
    #[test]
    fn rejected_backend_session_commit_stops_worker_and_descendants() {
        scenario("session_started");
    }
    #[test]
    fn rejected_delegated_text_commit_stops_worker_without_canonical_output() {
        scenario("text_delta");
    }
    #[test]
    fn rejected_terminal_commit_cold_recovers_started_delivery_as_outcome_unknown() {
        scenario("completed");
    }
}
