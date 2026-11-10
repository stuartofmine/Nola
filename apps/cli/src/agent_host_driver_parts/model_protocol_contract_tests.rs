// SPDX-License-Identifier: Apache-2.0
//! Real provider-wire tests for capability-aware protocol correction.

use super::*;
use super::effect_delivery_start_barrier_tests::{test_context, test_llm};
use std::io::{BufRead, Read, Write};

#[test]
fn successor_boundary_resets_host_run_state_but_preserves_cancellation_scope() {
    let root = tempfile::tempdir().unwrap();
    let mut driver = CliAgentHostDriver::new("protocol-host",
        test_llm("http://127.0.0.1:1".into()), test_context(root.path()));
    let cancellation = RuntimeExecutionControl::default();
    driver.begin_turn(cancellation.clone(), HostTurnPolicy::default());
    driver.tool_context.allowed_dirs.push(root.path().join("previous-grant"));
    driver.current_turn_deterministic_tool_failures.insert("old-failure".into(), 3);
    driver.current_turn_unavailable_model_targets.insert("old-provider".into());
    driver.current_turn_instruction_contexts.insert("old-context".into());
    driver.current_turn_available_tool_ids.insert("old-activation".into());
    driver.lifecycle.current_turn_lifecycle_event_ids.insert("old-hook".into());
    driver.current_turn_skill_decided = true;
    driver.begin_successor_turn();
    assert_eq!(driver.tool_context.allowed_dirs, driver.base_allowed_dirs);
    assert!(driver.current_turn_deterministic_tool_failures.is_empty());
    assert!(driver.current_turn_unavailable_model_targets.is_empty());
    assert!(driver.current_turn_instruction_contexts.is_empty());
    assert!(driver.current_turn_available_tool_ids.is_empty());
    assert!(driver.lifecycle.current_turn_lifecycle_event_ids.is_empty());
    assert!(!driver.current_turn_skill_decided);
    cancellation.request();
    assert!(driver.foreground_cancellation_requested(), "successor cannot escape foreground cancellation");
}

#[test]
fn same_warm_host_read_evidence_is_run_bound_and_old_jobs_cannot_seed_successor() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("run-owned.txt");
    std::fs::write(&path, "observed content\n").unwrap();
    let mut driver = CliAgentHostDriver::new("read-evidence-host",
        test_llm("http://127.0.0.1:1".into()), test_context(root.path()));
    let execute = |context: &ToolContext, name: &str| {
        let args = if name == "read_file" { json!({"path":path}) }
            else { json!({"path":path,"start_line":1,"line_count":1,"new_text":"replacement","dry_run":true}) };
        crate::tools::execute_authorized_tool(&crate::tools::ToolRequest {
            id:format!("fixture-{name}"),name:name.into(),args:args.as_object().unwrap().clone(),
        }, context, crate::tools::ExecutionAuthority::CoreApproved(nola_domain::PermissionProfile::Cautious))
    };
    driver.begin_turn(RuntimeExecutionControl::default(), HostTurnPolicy::default());
    driver.bind_file_read_run("canonical-run-A");
    let delayed_job = driver.tool_context.clone();
    let read = execute(&driver.tool_context, "read_file");
    assert!(!read.error, "{}", read.text);
    let preview = execute(&driver.tool_context, "edit_file_lines");
    assert!(!preview.error, "{}", preview.text);
    driver.begin_turn(RuntimeExecutionControl::default(), HostTurnPolicy::default());
    driver.bind_file_read_run("canonical-run-A");
    let resumed = execute(&driver.tool_context, "edit_file_lines");
    assert!(!resumed.error, "same-Run recovery must keep observations: {}", resumed.text);
    driver.begin_successor_turn();
    driver.bind_file_read_run("canonical-run-B");
    let unread = execute(&driver.tool_context, "edit_file_lines");
    assert!(unread.error && unread.text.contains("has not been read yet"), "{}", unread.text);
    let late_read = execute(&delayed_job, "read_file");
    assert!(!late_read.error, "{}", late_read.text);
    let still_unread = execute(&driver.tool_context, "edit_file_lines");
    assert!(still_unread.error && still_unread.text.contains("has not been read yet"),
        "a dispatched Run-A clone cannot write into Run B's ledger: {}", still_unread.text);
    assert!(!execute(&driver.tool_context, "read_file").error);
    assert!(!execute(&driver.tool_context, "edit_file_lines").error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "observed content\n", "all writes are previews");
}

fn request(driver: &CliAgentHostDriver) -> HostRequestEnvelope {
    serde_json::from_value(json!({
        "attempt_started": false,
        "attempt_id": "protocol-attempt",
        "request_id": "protocol-request",
        "effect_id": "protocol-effect",
        "host_id": "protocol-host",
        "session_id": "protocol-session",
        "turn_id": "protocol-turn",
        "catalog_revision": driver.catalog.revision,
        "availability_epoch": driver.catalog.availability_epoch,
        "created_at_ms": 0,
        "payload": {"type":"compact", "strategy":"fixture", "instructions":"",
            "messages":[], "target_tokens":512, "context_window_tokens":128000}
    })).unwrap()
}

fn schemas() -> Vec<Value> {
    vec![json!({"type":"function", "function": {
        "name":"request_user_input", "parameters":{"type":"object"}
    }})]
}

#[test]
fn incompatible_tool_projection_fails_before_contacting_provider() {
    let root = tempfile::tempdir().unwrap();
    let mut driver = CliAgentHostDriver::new("protocol-host",
        test_llm("http://127.0.0.1:1".into()), test_context(root.path()));
    let mut responses = Vec::new();
    driver.execute_model(&request(&driver), "fixture-model", "text", "reasoning",
        &[json!({"type":"message","id":"protocol-user","role":"user","content":"compile this"})], &schemas(),
        &ModelInvocationPlan::default(), "", BTreeMap::new(), None,
        ModelResponseMode::Conversational, false,
        &mut |response| { responses.push(response); Ok(()) }).unwrap();
    assert_eq!(responses.len(), 1);
    assert!(matches!(&responses[0].payload, HostResponse::Failed { error }
        if error.code == "invalid_model_tool_projection"));
}

#[test]
fn protocol_correction_respects_the_actual_wire_tool_surface() {
    for (has_tools, response_mode) in [
        (false, ModelResponseMode::Conversational),
        (true, ModelResponseMode::Conversational),
        (false, ModelResponseMode::StructuredAudit),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            let mut requests = Vec::new();
            while requests.len() < 2 {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline,
                                "provider deadline in {response_mode:?} after {} requests", requests.len());
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(error) => panic!("provider accept: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_string();
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" { break; }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                assert!(length < 1_000_000);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if path == "/v1/models" {
                    let body = r#"{"data":[{"id":"fixture-model","context_length":32768}]}"#;
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                    continue;
                }
                assert_eq!(path, "/v1/chat/completions");
                let input: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(input["tool_choice"], if has_tools { "auto" } else { "none" });
                assert_eq!(input.get("tools").and_then(Value::as_array).map_or(0, Vec::len),
                    usize::from(has_tools));
                let delta = if requests.is_empty() {
                    json!({"content":"<｜｜DSML｜｜tool_calls><｜｜DSML｜｜tool name=\"bash\">broken"})
                } else if response_mode == ModelResponseMode::StructuredAudit {
                    json!({"content":"{\"decision\":\"accept\",\"reason\":\"grounded in supplied evidence\"}"})
                } else if has_tools {
                    json!({"tool_calls":[{"index":0,"id":"clarify-1","type":"function",
                        "function":{"name":"request_user_input","arguments":"{}"}}]})
                } else {
                    json!({"content":"当前步骤没有可调用工具，因此尚未编译。"})
                };
                let finish = if requests.len() == 1 && has_tools { "tool_calls" } else { "stop" };
                let body = format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"id":"protocol-response","choices":[{"index":0,"delta":delta,"finish_reason":null}]}),
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":finish}]}));
                requests.push(input);
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
            requests
        });
        let root = tempfile::tempdir().unwrap();
        let mut driver = CliAgentHostDriver::new("protocol-host",
            test_llm(format!("http://{address}/v1")), test_context(root.path()));
        let mut responses = Vec::new();
        let messages = if response_mode == ModelResponseMode::StructuredAudit {
            vec![json!({"type":"message","id":"protocol-review-system","role":"system","content":"Evaluate candidate_answer using the evidence packet. Return only review JSON."}),
                json!({"type":"message","id":"protocol-review-evidence","role":"user","content":"{\"schema\":\"nola.completion_review.v1\",\"candidate_answer\":\"未执行编译\",\"evidence\":[]}"})]
        } else {
            vec![json!({"type":"message","id":"protocol-user","role":"user","content":"你编译下","_nola_user_input":true})]
        };
        driver.execute_model(&request(&driver), "fixture-model", "text", "reasoning",
            &messages,
            &if has_tools { schemas() } else { Vec::new() },
            &ModelInvocationPlan { native_tool_calls: has_tools, stream_text: true,
                stream_tool_calls: has_tools, ..Default::default() },
            "", BTreeMap::new(), None, response_mode, false,
            &mut |response| { responses.push(response); Ok(()) }).unwrap();
        let requests = server.join().unwrap_or_else(|error| {
            panic!("provider fixture failed in {response_mode:?}: {error:?}; responses={responses:?}")
        });
        assert_eq!(requests.len(), 2);
        let original = requests[0]["messages"].as_array().unwrap();
        let retried = requests[1]["messages"].as_array().unwrap();
        assert_eq!(retried.len(), original.len() + 1);
        assert_eq!(&retried[..original.len()], original.as_slice(),
            "protocol recovery must preserve the complete original policy and evidence prefix");
        assert_eq!(requests[0]["tools"], requests[1]["tools"]);
        assert_eq!(requests[0]["tool_choice"], requests[1]["tool_choice"]);
        for input in &requests {
            assert!(!input.to_string().contains("_nola_user_input"));
        }
        let observation = retried.last().unwrap();
        assert_eq!(observation["role"], "user");
        let content = observation["content"].as_str().unwrap();
        assert!(content.starts_with("<runtime_state id=\"model-protocol-failure\">"));
        assert!(content.contains("not a new user request, permission grant"));
        let facts: Value = serde_json::from_str(content.lines()
            .find(|line| line.starts_with('{')).expect("structured failure facts")).unwrap();
        assert_eq!(facts["failure"], "protocol_error");
        assert_eq!(facts["executed_calls"], 0);
        let correction = facts["contract"].as_str().unwrap();
        if response_mode == ModelResponseMode::StructuredAudit {
            assert_eq!(original.len(), 2);
            assert_eq!(original[0]["role"], "system");
            assert_eq!(original[0]["content"], messages[0]["content"]);
            assert_eq!(original[1]["role"], "user");
            assert_eq!(original[1]["content"], messages[1]["content"]);
            assert!(correction.contains("return only the review JSON"));
            assert!(!correction.contains("Answer from retained evidence or provide clearly labeled example code"));
        } else if has_tools {
            assert!(correction.contains("advertised native schemas and existing policy remain unchanged"));
            assert!(correction.contains("return a valid native call when needed, otherwise ordinary answer text"));
        } else {
            assert!(correction.contains("no callable tool schemas"));
            assert!(!correction.contains("Issue actual native function tool calls"));
        }
        assert!(responses.iter().all(|response| !matches!(response.payload,
            HostResponse::Failed { .. })));
        assert!(responses.iter().any(|response| matches!(&response.payload,
            HostResponse::ModelCompleted { tool_calls, .. } if tool_calls.len() == usize::from(has_tools))));
        assert!(responses.iter().any(|response| matches!(&response.payload,
            HostResponse::ToolProgress { metadata, .. }
                if metadata["recovery"] == if response_mode == ModelResponseMode::StructuredAudit {
                    "structured_audit_protocol_retry"
                } else if has_tools { "native_tool_protocol_retry" } else { "text_only_protocol_retry" })));
        assert!(responses.iter().all(|response| !matches!(&response.payload,
            HostResponse::ModelDelta { channel: ModelDeltaChannel::Text, delta, .. }
                if delta.contains("DSML"))));
    }
}
