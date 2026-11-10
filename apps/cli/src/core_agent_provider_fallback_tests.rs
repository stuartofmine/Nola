// SPDX-License-Identifier: Apache-2.0
// Real provider fallback through CLI Host and in-process Application persistence.
#[cfg(test)]
mod provider_fallback_application_tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};

    const CLEAN: &str = "Clean fallback answer.";

    #[derive(serde::Deserialize)]
    struct AttemptFacts {
        id: String,
        run_id: String,
        invocation_id: String,
        effect_id: String,
        text_part_id: String,
        predecessor_id: Option<String>,
        state: String,
        provider: Option<String>,
        model: Option<String>,
        started_at_ms: Option<i64>,
        completed_at_ms: Option<i64>,
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

    fn provider_identity(endpoint: &str) -> String {
        use sha2::Digest;
        format!(
            "openai-compatible.chat-completions:{:x}",
            sha2::Sha256::digest(endpoint.as_bytes())
        )
    }

    #[test]
    fn fallback_sampling_has_committed_distinct_target_attempts_and_clean_cold_history() {
        let _environment = crate::store::test_env_lock();
        let root =
            std::env::temp_dir().join(format!("nola-provider-fallback-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let fixture = Fixture {
            root: root.clone(),
            session: format!("fallback-{}", uuid::Uuid::new_v4()),
            stop: stop.clone(),
            environment: ["NOLA_HOME", "NOLA_DB_PATH", "NOLA_LLM_MAX_RETRIES"]
                .into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect(),
        };
        let database = root.join("canonical.sqlite");
        std::env::set_var("NOLA_HOME", &root);
        std::env::set_var("NOLA_DB_PATH", &database);
        std::env::set_var("NOLA_LLM_MAX_RETRIES", "0");
        let listeners = [
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
            std::net::TcpListener::bind("127.0.0.1:0").unwrap(),
        ];
        let endpoints = listeners
            .iter()
            .map(|listener| {
                listener.set_nonblocking(true).unwrap();
                format!("http://{}/v1", listener.local_addr().unwrap())
            })
            .collect::<Vec<_>>();
        let targets = endpoints
            .iter()
            .map(|endpoint| provider_identity(endpoint))
            .collect::<Vec<_>>();
        let server_database = database.clone();
        let server_targets = targets.clone();
        let samples = Arc::new(Mutex::new(Vec::<(usize, String, String)>::new()));
        let server_samples = samples.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(40);
            while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
                for (target, listener) in listeners.iter().enumerate() {
                    let (mut stream, _) = match listener.accept() {
                        Ok(value) => value,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                        Err(error) => panic!("provider accept: {error}"),
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                    let mut first = String::new();
                    if reader.read_line(&mut first).unwrap() == 0 {
                        continue;
                    }
                    let path = first.split_whitespace().nth(1).unwrap().to_owned();
                    let mut length = 0;
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header == "\r\n" {
                            break;
                        }
                        if let Some(value) =
                            header.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                    }
                    assert!(length < 2_000_000);
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).unwrap();
                    let (status, mime, output) = if path == "/v1/models" {
                        ("200 OK", "application/json", serde_json::json!({"data":[{"id":"primary-model","context_length":32768}]}).to_string())
                    } else {
                        assert_eq!(path, "/v1/chat/completions");
                        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        let expected_model = if target == 0 {
                            "primary-model"
                        } else {
                            "fallback-model"
                        };
                        assert_eq!(body["model"], expected_model);
                        // The actual HTTP arrival must already have its committed Running ledger row.
                        let db = rusqlite::Connection::open(&server_database).unwrap();
                        let raw: String = db
                            .query_row(
                                "SELECT record_json FROM run_attempts WHERE state='running'",
                                [],
                                |row| row.get(0),
                            )
                            .unwrap();
                        let record: AttemptFacts = serde_json::from_str(&raw).unwrap();
                        assert_eq!(
                            record.provider.as_deref(),
                            Some(server_targets[target].as_str())
                        );
                        assert_eq!(record.model.as_deref(), Some(expected_model));
                        server_samples.lock().unwrap().push((
                            target,
                            record.id,
                            expected_model.into(),
                        ));
                        if target == 0 {
                            (
                                "503 Service Unavailable",
                                "application/json",
                                serde_json::json!({"error":{"message":"primary unavailable"}})
                                    .to_string(),
                            )
                        } else {
                            // Include continuation identity to cover the real
                            // cross-model fallback path. It must not be attached
                            // to the Turn configured for `primary-model`.
                            let delta = serde_json::json!({"id":"fallback-response","model":"fallback-model","choices":[{"index":0,"delta":{"content":CLEAN},"finish_reason":null}]});
                            let done = serde_json::json!({"id":"fallback-response","model":"fallback-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
                            (
                                "200 OK",
                                "text/event-stream",
                                format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
                            )
                        }
                    };
                    write!(stream, "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{output}", output.len()).unwrap();
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let mut config = crate::config::Config::from_args(vec![
            "nola".into(),
            "--session".into(),
            fixture.session.clone(),
            "--project".into(),
            root.to_string_lossy().into_owned(),
            "--endpoint".into(),
            endpoints[0].clone(),
            "--api-key".into(),
            "fixture".into(),
            "--model".into(),
            "primary-model".into(),
        ])
        .and_then(|config| config.host_invocation_request())
        .unwrap();
        config.session.save_session = true;
        config.session.supports_reviewed_text_parts = false;
        config.tool_settings.tools_enabled = false;
        config.tool_settings.skill_dirs = Some(vec![]);
        config.runtime.llm_fallbacks = serde_json::from_value(
            serde_json::json!([{"endpoint":endpoints[1],"model":"fallback-model"}]),
        )
        .unwrap();
        let mut emitter = Emitter::with_event_sink(|_| {});
        let outcome = run_agent_core(&config, "Answer briefly", &mut emitter);
        fixture.stop.store(true, Ordering::SeqCst);
        server.join().unwrap();
        assert!(outcome.unwrap().is_terminal());
        let samples = samples.lock().unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(
            samples.iter().map(|sample| sample.0).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_ne!(samples[0].1, samples[1].1);
        let db = rusqlite::Connection::open(&database).unwrap();
        let mut query = db.prepare("SELECT record_json FROM run_attempts").unwrap();
        let attempts = query
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|row| serde_json::from_str::<AttemptFacts>(&row.unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(attempts.len(), samples.len());
        let first = attempts
            .iter()
            .find(|attempt| attempt.id == samples[0].1)
            .unwrap();
        let second = attempts
            .iter()
            .find(|attempt| attempt.id == samples[1].1)
            .unwrap();
        assert_eq!(first.predecessor_id, None);
        assert_eq!(second.predecessor_id.as_deref(), Some(first.id.as_str()));
        assert_eq!(first.state, "superseded");
        assert_eq!(second.state, "succeeded");
        assert_eq!(first.effect_id, second.effect_id);
        assert_eq!(first.run_id, second.run_id);
        assert_eq!(first.invocation_id, second.invocation_id);
        assert_ne!(first.text_part_id, second.text_part_id);
        for (index, attempt) in [first, second].into_iter().enumerate() {
            assert_eq!(attempt.provider.as_deref(), Some(targets[index].as_str()));
            assert_eq!(attempt.model.as_deref(), Some(samples[index].2.as_str()));
            assert!(attempt.started_at_ms.is_some() && attempt.completed_at_ms.is_some());
        }
        invalidate_core_agent_session(&fixture.session).unwrap();
        // Canonical text lives in owned chunks, not the legacy scalar column.
        // A fresh bounded Store reader must restore only the winning attempt.
        let cold_store = nola_store::NolaStore::open(&database).unwrap();
        let window = cold_store
            .load_conversation_message_window(&fixture.session, None, 16, 32768)
            .unwrap()
            .unwrap();
        assert!(!window.has_more);
        assert!(window.deferred_text_parts.is_empty());
        let parts = window
            .conversation
            .messages
            .iter()
            .filter(|message| message.role == "assistant")
            .flat_map(|message| {
                message.parts.iter().filter_map(|part| {
                    if part.kind != "text" || part.content_text.is_empty() {
                        return None;
                    }
                    assert_eq!(message.run_id.as_deref(), Some(second.run_id.as_str()));
                    Some((part.id.clone(), part.content_text.clone()))
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(parts, vec![(second.text_part_id.clone(), CLEAN.into())]);
        let owner = cold_store
            .model_text_part_owner(&fixture.session, &second.text_part_id)
            .unwrap()
            .unwrap();
        assert_eq!(owner.run_id, second.run_id);
        assert_eq!(owner.attempt_id, second.id);
        assert_eq!(owner.text_part_id, second.text_part_id);
        assert!(cold_store
            .model_text_part_owner("another-conversation", &second.text_part_id)
            .unwrap()
            .is_none());
        assert_eq!(
            db.query_row::<i64, _, _>(
                "SELECT COUNT(*) FROM effect_outbox WHERE status='pending'",
                [],
                |row| row.get(0)
            )
            .unwrap(),
            0
        );
        let checkpoint = SessionStore::load_agent_checkpoint_for_session(&fixture.session)
            .unwrap()
            .unwrap();
        assert!(serde_json::to_string(&checkpoint).unwrap().contains(CLEAN));
        assert!(
            crate::application_port::CliApplicationPort::persistent_default()
                .unwrap()
                .active_run(&fixture.session)
                .unwrap()
                .is_none()
        );
    }
}
