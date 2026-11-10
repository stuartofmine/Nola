// SPDX-License-Identifier: Apache-2.0
// Real HTTP provider -> CLI Host -> in-process Application/SQLite regression.
// This does not exercise Service IPC or claim a complete durable Attempt ledger.
#[cfg(test)]
mod provider_retry_application_tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const OLD: &str = "OLD_RETRY_REASONING_";
    const CLEAN: &str = "Clean replacement answer.";

    fn committed_first_attempt_reasoning(path: &std::path::Path) -> String {
        let db = rusqlite::Connection::open(path).unwrap();
        let mut rows = db.prepare("SELECT json_extract(payload_json,'$.payload.delta') FROM host_event_inbox
            WHERE json_extract(payload_json,'$.payload.type')='model_delta'
              AND json_extract(payload_json,'$.payload.channel')='reasoning'
              AND json_extract(payload_json,'$.attempt_id')=(SELECT id FROM run_attempts WHERE json_extract(record_json,'$.predecessor_id') IS NULL ORDER BY rowid LIMIT 1)
            ORDER BY sequence").unwrap();
        let deltas = rows.query_map([], |row| row.get::<_, String>(0)).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        deltas.concat()
    }

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

    fn canonical_rows(path: &std::path::Path) -> Vec<Vec<String>> {
        let db = rusqlite::Connection::open(path).unwrap();
        [
            "core_checkpoints",
            "core_checkpoint_bodies",
            "core_checkpoint_replay_bases",
            "messages",
            "message_parts",
            "host_event_inbox",
            "effect_outbox",
            "runs",
            "run_invocation_payloads",
            "run_attempts",
            "run_attempt_deliveries",
            "assistant_journal_heads",
            "assistant_journal_records",
            "assistant_journal_parts",
            "assistant_journal_part_versions",
            "assistant_journal_chunks",
            "attempt_audit",
        ]
        .into_iter()
        .map(|table| {
            let mut query = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let columns = query.column_count();
            query
                .query_map([], |row| {
                    Ok((0..columns)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .collect()
    }

    fn scenario(mode: u8) {
        let reject_reset = mode == 1;
        let reject_initial = mode == 2;
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-provider-retry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database = root.join("canonical.sqlite");
        let stop = Arc::new(AtomicBool::new(false));
        let fixture = Fixture {
            root: root.clone(),
            session: format!("retry-{}", uuid::Uuid::new_v4()),
            stop: stop.clone(),
            environment: ["NOLA_HOME", "NOLA_DB_PATH", "NOLA_LLM_MAX_RETRIES"]
                .into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect(),
        };
        std::env::set_var("NOLA_HOME", &root);
        std::env::set_var("NOLA_DB_PATH", &database);
        std::env::set_var("NOLA_LLM_MAX_RETRIES", "1");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let posts = Arc::new(AtomicUsize::new(0));
        let count = posts.clone();
        let baseline = Arc::new(Mutex::new(None));
        let server_database = database.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(40);
            while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture accept: {error}"),
                };
                // On macOS an accepted stream can inherit the listener's
                // nonblocking mode. The fixture polls the listener, but the
                // HTTP request itself is consumed with blocking reads.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let path = first.split_whitespace().nth(1).unwrap().to_owned();
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
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let (mime, response) = if path == "/v1/models" {
                    (
                        "application/json",
                        serde_json::json!({"data":[{"id":"fixture-model","context_length":32768}]})
                            .to_string(),
                    )
                } else {
                    assert_eq!(path, "/v1/chat/completions");
                    let ordinal = count.fetch_add(1, Ordering::SeqCst);
                    assert!(ordinal < 3, "unplanned provider attempt beyond retry/audit");
                    if ordinal == 0 {
                        if reject_reset {
                            rusqlite::Connection::open(&server_database).unwrap().execute_batch(
                                "CREATE TRIGGER reject_model_reset BEFORE INSERT ON host_event_inbox WHEN json_extract(NEW.payload_json,'$.payload.type')='model_attempt_started' BEGIN SELECT RAISE(ABORT,'model reset admission fault'); END"
                            ).unwrap();
                        }
                        let reasoning = serde_json::json!({"choices":[{"index":0,"delta":{"reasoning_content":OLD.repeat(12)},"finish_reason":null}]});
                        let partial_tool = serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"discarded-partial-call","type":"function","function":{"name":"write_file","arguments":"{\"path\":"}}]},"finish_reason":null}]});
                        // Valid SSE frames followed by EOF with neither finish_reason nor [DONE].
                        (
                            "text/event-stream",
                            format!("data: {reasoning}\n\ndata: {partial_tool}\n\n"),
                        )
                    } else {
                        let delta = if ordinal == 2 {
                            serde_json::json!({"content":r#"{"decision":"accept","outcome":"answered","constraints":[],"items":[{"label":"Fixture task requirement","eligibility":"eligible","evidence":"Observed fixture evidence supports the candidate"}],"violations":[]}"#})
                        } else {
                            serde_json::json!({"tool_calls":[{"index":0,"id":"review-clean-candidate","type":"function","function":{"name":"request_completion_review","arguments":serde_json::json!({"candidate_answer":CLEAN}).to_string()}}]})
                        };
                        let clean = serde_json::json!({"choices":[{"index":0,"delta":delta,"finish_reason":null}]});
                        let end = serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":if ordinal == 2 {"stop"} else {"tool_calls"}}]});
                        (
                            "text/event-stream",
                            format!("data: {clean}\n\ndata: {end}\n\ndata: [DONE]\n\n"),
                        )
                    }
                };
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).unwrap();
            }
        });
        let mut config = crate::config::Config::from_args(vec![
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
        .and_then(|config| config.host_invocation_request())
        .unwrap();
        config.session.save_session = true;
        config.session.supports_reviewed_text_parts = false;
        config.tool_settings.tools_enabled = true;
        config.runtime.model_capabilities.reasoning = Some(true);
        config.runtime.model_capabilities.stream_reasoning = Some(true);
        config.runtime.model_capabilities.native_tool_calls = Some(true);
        config.tool_settings.tool_allowlist = Some(vec!["write_file".into()]);
        config.tool_settings.allowed_dirs = vec![root.clone()];
        config.tool_settings.skill_dirs = Some(Vec::new());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let committed_retry_baseline = baseline.clone();
        let retry_database = database.clone();
        let mut emitter = Emitter::with_event_sink(move |event| {
            // SamplingStarted first flushes the old attempt's buffered body.
            // Pin the last accepted old-output boundary, not the earlier retry
            // progress notification. A rejected reset must change none of it.
            if event["type"].as_str() == Some("assistant_journal_committed_v1")
                && committed_retry_baseline.lock().unwrap().is_none()
                && committed_first_attempt_reasoning(&retry_database) == OLD.repeat(12)
            {
                *committed_retry_baseline.lock().unwrap() = Some(canonical_rows(&retry_database));
            }
            sink.lock().unwrap().push(event);
        });
        let host = acquire_core_agent_host(&config, Vec::new()).unwrap();
        let audit = rusqlite::Connection::open(&database).unwrap();
        audit.execute_batch("CREATE TABLE attempt_audit(id TEXT,state TEXT);
            CREATE TRIGGER audit_attempt_insert AFTER INSERT ON run_attempts BEGIN INSERT INTO attempt_audit VALUES(NEW.id,NEW.state); END;
            CREATE TRIGGER audit_attempt_update AFTER UPDATE ON run_attempts WHEN OLD.state != NEW.state BEGIN INSERT INTO attempt_audit VALUES(NEW.id,NEW.state); END;").unwrap();
        if reject_initial {
            audit.execute_batch("CREATE TRIGGER reject_model_initial BEFORE INSERT ON host_event_inbox WHEN json_extract(NEW.payload_json,'$.payload.type')='model_attempt_started' BEGIN SELECT RAISE(ABORT,'initial model admission fault'); END").unwrap();
        }
        let result = host.lock().unwrap().run_turn_items(
            &config,
            host_user_input_items(&config, "Answer without changing files"),
            nola_core::context_engine::ContextPlatform::Unknown,
            Vec::new(),
            RuntimeExecutionControl::default(),
            &mut emitter,
        );
        fixture.stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
        if reject_initial {
            let error = result.unwrap_err();
            assert!(error.contains("initial model admission fault"), "{error}");
            assert_eq!(
                posts.load(Ordering::SeqCst),
                0,
                "Initial admission failure cannot make a sampling HTTP request"
            );
            let records = attempt_records(&database);
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].state, "prepared");
            assert!(records[0].provider.is_none() && records[0].started_at_ms.is_none());
            assert_eq!(audit.query_row::<i64,_,_>("SELECT COUNT(*) FROM host_event_inbox WHERE json_extract(payload_json,'$.payload.type')='model_attempt_started'", [], |r|r.get(0)).unwrap(), 0);
            return;
        }
        assert_eq!(committed_first_attempt_reasoning(&database), OLD.repeat(12),
            "fixture must durably accept the entire first-attempt reasoning before reset");
        if reject_reset {
            let error = result.unwrap_err();
            assert!(
                error.contains("model reset admission fault"),
                "unexpected error: {error}"
            );
            assert_eq!(
                posts.load(Ordering::SeqCst),
                1,
                "Rejected reset must stop before retry HTTP"
            );
            assert_eq!(
                canonical_rows(&database),
                baseline.lock().unwrap().as_ref().expect("complete old-attempt output publication before rejected sampling").clone(),
                "Rejected reset cannot publish checkpoint, Parts or accepted response state"
            );
        } else {
            assert!(result.unwrap().is_terminal());
            assert_eq!(posts.load(Ordering::SeqCst), 3);
            let records = attempt_records(&database);
            assert_eq!(records.len(), 3);
            let first = records.iter().find(|r| r.state == "superseded").unwrap();
            let second = records
                .iter()
                .find(|r| r.predecessor_id.as_deref() == Some(first.id.as_str()))
                .unwrap();
            assert_ne!(first.id, second.id);
            assert_eq!(first.state, "superseded");
            assert!(first.failure.is_some(), "supersession must retain the actual failed sampling evidence");
            assert_ne!(first.failure.as_ref().unwrap()["code"], "model_resampled",
                "retry progress records the real provider failure before supersession");
            assert_eq!(second.state, "succeeded");
            assert_eq!(first.effect_id, second.effect_id);
            assert_eq!(first.invocation_id, second.invocation_id);
            assert_eq!(first.run_id, second.run_id);
            assert_ne!(first.text_part_id, second.text_part_id);
            assert_ne!(first.reasoning_part_id, second.reasoning_part_id);
            use sha2::Digest;
            let provider = format!(
                "openai-compatible.chat-completions:{:x}",
                sha2::Sha256::digest(config.model_settings.endpoint.trim().trim_end_matches('/').as_bytes())
            );
            for record in &records {
                assert_eq!(record.provider.as_deref(), Some(provider.as_str()));
                assert_eq!(record.model.as_deref(), Some("fixture-model"));
                assert!(record.started_at_ms.is_some() && record.completed_at_ms.is_some());
            }
            let mut states = audit
                .prepare("SELECT state FROM attempt_audit WHERE id=?1 ORDER BY rowid")
                .unwrap();
            let states_for = |id: &str, statement: &mut rusqlite::Statement<'_>| {
                statement
                    .query_map([id], |row| row.get::<_, String>(0))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            assert_eq!(
                states_for(&first.id, &mut states),
                vec!["prepared", "running", "superseded"]
            );
            assert_eq!(
                states_for(&second.id, &mut states),
                vec!["running", "succeeded"]
            );

            let snapshot = serde_json::to_string(&host.lock().unwrap().runtime.control()).unwrap();
            assert!(
                !snapshot.contains(OLD),
                "hot canonical projection retained failed-attempt reasoning"
            );
            drop(host);
            invalidate_core_agent_session(&fixture.session).unwrap();
            let checkpoint = SessionStore::load_agent_checkpoint_for_session(&fixture.session)
                .unwrap()
                .unwrap();
            let raw = serde_json::to_string(&checkpoint).unwrap();
            assert!(
                !raw.contains(OLD),
                "cold checkpoint retained failed-attempt reasoning"
            );
            assert!(raw.contains(CLEAN));
            let db = rusqlite::Connection::open(&database).unwrap();
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
                assert!(
                    !text.contains(OLD) && !json.contains(OLD),
                    "canonical Part retained failed-attempt reasoning"
                );
            }
        }
    }

    #[derive(serde::Deserialize)]
    struct AttemptFacts {
        id: String,
        run_id: String,
        invocation_id: String,
        effect_id: String,
        predecessor_id: Option<String>,
        state: String,
        provider: Option<String>,
        model: Option<String>,
        text_part_id: String,
        reasoning_part_id: String,
        started_at_ms: Option<i64>,
        completed_at_ms: Option<i64>,
        failure: Option<serde_json::Value>,
    }

    fn attempt_records(path: &std::path::Path) -> Vec<AttemptFacts> {
        let db = rusqlite::Connection::open(path).unwrap();
        let mut query = db
            .prepare("SELECT record_json FROM run_attempts ORDER BY id")
            .unwrap();
        query
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|r| serde_json::from_str(&r.unwrap()).unwrap())
            .collect()
    }

    #[test]
    fn initial_attempt_admission_failure_prevents_provider_sampling() {
        scenario(2);
    }

    #[test]
    fn provider_retry_replaces_published_reasoning_in_canonical_and_cold_history() {
        scenario(0);
    }
    #[test]
    fn rejected_restart_transaction_stops_before_second_provider_request() {
        scenario(1);
    }
}
