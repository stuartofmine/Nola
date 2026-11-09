// SPDX-License-Identifier: Apache-2.0
use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

#[test]
fn sampling_measurement_does_not_promote_partial_choices_to_success() {
    use nola_measurement::Outcome;
    for (reasons, expected) in [
        (vec!["stop", "end_turn", "stop_sequence", "completed", "tool_calls", "tool_use", "function_call"], Outcome::Succeeded),
        (vec!["stream_incomplete", "length", "max_tokens", "max_output_tokens", "pause_turn", "", "future_reason"], Outcome::Incomplete),
        (vec!["protocol_error"], Outcome::Failed),
        (vec!["content_filter", "refusal"], Outcome::Rejected),
    ] {
        for reason in reasons {
            let choice = StreamedChoice {
                finish_reason: reason.into(), content: "preserved partial or complete text".into(),
                ..Default::default()
            };
            assert_eq!(choice.sampling_measurement_outcome(), expected, "{reason}");
            assert_eq!(choice.content, "preserved partial or complete text");
            assert_eq!(choice.finish_reason, reason);
        }
    }
}

fn config(provider: &str, endpoint: String) -> LlmConfig {
    LlmConfig {
        siwc: None, native_model_route: None,
        compaction_output_budget: None,
        chat_output_token_field: Default::default(),
        provider: Some(provider.into()),
        fallbacks: vec![],
        endpoint,
        api_key: "private-fixture-key".into(),
        model: "fixture-model".into(),
        temperature: 0.2,
        top_p: 0.9,
        max_output_tokens: 8192,
        max_retries: 0,
        thinking_enabled: true, reasoning_summary_supported: None,
        reasoning_selection: None,
        sampling_override: false,
        generation_options: Default::default(),
        cancellation: Default::default(),
        openai_prompt_cache: true,
        openai_prompt_cache_key: Some("stable-session-key".into()),
        openai_prompt_cache_key_prefix: None,
        openai_prompt_cache_retention: None,
        openai_prompt_cache_history_breakpoint: false,
        openai_prompt_cache_history_breakpoint_endpoints: None,
        parallel_tool_calls: Some(false),
    }
}

#[test]
fn cache_hit_rate_normalizes_protocol_denominators_without_counting_writes_as_hits() {
    let usage = LlmUsage { input_tokens: Some(100), cached_input_tokens:Some(800), cache_write_input_tokens:Some(100), ..Default::default() };
    let measured = usage.cache_usage(true, true);
    assert_eq!(measured.input_tokens, Some(1000));
    assert_eq!(measured.hit_rate_bps, Some(8000));
    assert!(measured.is_compaction);
    assert_eq!(usage.cache_usage(false, false).hit_rate_bps, None);
    let chat = LlmUsage { input_tokens: Some(1000), ..usage.clone() };
    assert_eq!(chat.cache_usage(false, false).hit_rate_bps, Some(8000));
    let unknown = LlmUsage { cache_write_input_tokens:None, ..usage };
    assert_eq!(unknown.cache_usage(true, false).hit_rate_bps, None);
}

fn history() -> Vec<Value> {
    vec![
        json!({"role":"system","content":"Frozen system instruction."}),
        json!({"role":"user","content":"archived evidence ".repeat(2500)}),
        json!({"role":"assistant","content":"Keep this recent answer verbatim."}),
        json!({"role":"user","content":"Continue the work."}),
    ]
}

#[test]
fn compaction_continuation_preserves_each_protocols_physical_prefix() {
    for provider in ["openai", "openai-responses", "anthropic"] {
        let cfg = config(provider, "http://127.0.0.1:1/v1".into());
        let protocol = resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint);
        let body = match protocol {
            ModelProtocol::SiwcResponses => unreachable!("API tool fixture excludes SIWC"),
            ModelProtocol::Responses => super::super::responses::responses_body(&cfg, &history(), &tools(), "auto"),
            ModelProtocol::Anthropic => super::super::provider_protocols::anthropic_request_body(&cfg, &history(), &tools(), "auto"),
            ModelProtocol::ChatCompletions => super::super::streaming::openai_request_body(&cfg, &history(), &tools(), "auto"),
        }.unwrap();
        let key = if protocol == ModelProtocol::Responses { "input" } else { "messages" };
        let snapshot = PreparedModelRequest { config:cfg, body:body.clone(), completed_response:vec![], started_at:Instant::now() };
        let mut choice = StreamedChoice { content:"partial checkpoint".into(), finish_reason:"length".into(),
            usage:LlmUsage { input_tokens: Some(12000), ..Default::default() }, ..Default::default() };
        if protocol == ModelProtocol::Responses {
            choice.response_items = vec![json!({"type":"reasoning","encrypted_content":"opaque"}),
                json!({"type":"message","role":"assistant","status":"incomplete","content":[{"type":"output_text","text":"partial checkpoint","annotations":[]}]})];
        }
        let continued = snapshot.continue_compaction(&choice,128000).unwrap();
        let rewritten = snapshot.rewrite_compaction_summary(&choice,128000,512).unwrap();
        for request in [&continued, &rewritten] {
            assert!(request.body[key].as_array().unwrap().last().unwrap().to_string()
                .contains(nola_core::agent::compaction_planner::COMPACTION_EVIDENCE_CONTRACT));
        }
        let mut rewrite_prefix = rewritten.body.clone();
        rewrite_prefix[key] = body[key].clone();
        assert_eq!(rewrite_prefix, body);
        assert!(rewritten.body.to_string().contains("2048 Unicode characters"));
        assert!(rewritten.body.to_string().contains("including headings and whitespace"));
        assert!(rewritten.body.to_string().contains("Remove unnecessary headings"));
        let count = body[key].as_array().unwrap().len();
        assert_eq!(&continued.body[key].as_array().unwrap()[..count],body[key].as_array().unwrap());
        let mut normalized = continued.body.clone(); normalized[key] = body[key].clone();
        assert_eq!(normalized,body);
        if protocol == ModelProtocol::Responses {
            assert_eq!(continued.body[key][count],choice.response_items[0]);
            assert_eq!(continued.body[key][count+1],choice.response_items[1]);
        }
        assert!(snapshot.continue_compaction(&choice,16384).is_err());
        choice.content.clear();
        assert!(snapshot.continue_compaction(&choice,128000).is_err());
    }
}

fn tools() -> Vec<Value> {
    vec![
        json!({"type":"function","function":{"name":"read_file","description":"Read a file",
        "parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}}),
    ]
}

#[test]
fn runtime_budget_tail_preserves_hot_fork_for_selected_last_user_and_tool_result() {
    for provider in ["openai", "openai-responses", "anthropic"] {
        for tool_result in [false, true] {
            let cfg = config(provider, "http://127.0.0.1:1/v1".into());
            let protocol = resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint);
            let mut canonical = vec![json!({"role":"system","content":"Frozen policy."}),
                json!({"role":"user","content":"Selected original user request."})];
            if tool_result {
                canonical.push(json!({"role":"assistant","content":"", "tool_calls":[{
                    "id":"call-observed", "type":"function", "function":{
                        "name":"read_file", "arguments":"{\"path\":\"observed.txt\"}"}}]}));
                canonical.push(json!({"role":"tool","tool_call_id":"call-observed","content":"Observed exact result."}));
            }
            let archive = canonical[1..].to_vec();
            for message in &mut canonical {
                message["_nola_user_input"] = json!(message["role"] == "user");
            }
            let mut physical = canonical.clone();
            physical.push(json!({"role":"user","_nola_user_input":false,"content":"<runtime_state id=\"external-execution-budget\">remaining_ms=50000</runtime_state>"}));
            let body = match protocol {
                ModelProtocol::SiwcResponses => unreachable!("API tool fixture excludes SIWC"),
                ModelProtocol::Responses => super::super::responses::responses_body(&cfg, &physical, &tools(), "auto"),
                ModelProtocol::Anthropic => super::super::provider_protocols::anthropic_request_body(&cfg, &physical, &tools(), "auto"),
                ModelProtocol::ChatCompletions => super::super::streaming::openai_request_body(&cfg, &physical, &tools(), "auto"),
            }.unwrap();
            assert!(!body.to_string().contains("_nola_user_input"), "{provider}: private provenance leaked");
            let key = if protocol == ModelProtocol::Responses {"input"} else {"messages"};
            let snapshot = PreparedModelRequest { config:cfg.clone(), body:body.clone(), completed_response:vec![], started_at:Instant::now() };
            let restored = PreparedModelRequest::restore(&snapshot.durable_value(), &cfg).unwrap();
            let fork = restored.fork(&archive, "Summarize selected evidence", 1024, 128000,
                &LlmUsage {input_tokens: Some(5000), cached_input_tokens: Some(0), cache_write_input_tokens: Some(0), ..Default::default()}, Default::default()).unwrap();
            let mut prefix = fork.body.clone();
            let instruction = prefix[key].as_array_mut().unwrap().pop().unwrap();
            assert_eq!(prefix, body, "{provider}: preserve every physical block and cache marker");
            assert!(prefix.to_string().contains("remaining_ms=50000"));
            if protocol == ModelProtocol::Anthropic {
                let instruction = instruction["content"].as_str().unwrap();
                let last = if tool_result {3} else {1};
                assert!(instruction.contains(&format!("message {last} blocks 1 through 1")));
                assert!(!instruction.contains(&format!("message {last} blocks 1 through 2")));
                assert!(instruction.contains("all other blocks in the same message remain verbatim"));
            }
        }
    }
}

#[test]
fn private_user_provenance_does_not_change_any_provider_body_or_source_alignment() {
    for provider in ["openai", "openai-responses", "anthropic"] {
        let cfg = config(provider, "http://127.0.0.1:1/v1".into());
        let protocol = resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint);
        let plain = history();
        let mut tagged = plain.clone();
        for message in &mut tagged {
            message["_nola_user_input"] = json!(message["role"] == "user");
        }
        let project = |messages: &[Value]| match protocol {
            ModelProtocol::SiwcResponses => unreachable!("API tool fixture excludes SIWC"),
            ModelProtocol::Responses => super::super::responses::responses_body(&cfg, messages, &tools(), "auto"),
            ModelProtocol::Anthropic => super::super::provider_protocols::anthropic_request_body(&cfg, messages, &tools(), "auto"),
            ModelProtocol::ChatCompletions => super::super::streaming::openai_request_body(&cfg, messages, &tools(), "auto"),
        };
        let body = project(&tagged).unwrap();
        assert_eq!(body, project(&plain).unwrap(), "{provider}");
        let snapshot = PreparedModelRequest {config:cfg.clone(),body,completed_response:vec![],started_at:Instant::now()};
        let usage = LlmUsage {input_tokens: Some(5000), cached_input_tokens: Some(0), cache_write_input_tokens: Some(0),..Default::default()};
        let plain_fork = snapshot.fork(&plain[1..2], "summary", 1024,128000,&usage,Default::default()).unwrap();
        let tagged_fork = snapshot.fork(&tagged[1..2], "summary",1024,128000,&usage,Default::default()).unwrap();
        assert_eq!(plain_fork.body, tagged_fork.body, "{provider}: metadata changed selection");
    }
}

#[test]
fn anthropic_block_selection_rejects_ambiguous_and_partial_text_matches() {
    let cfg = config("anthropic", "http://127.0.0.1:1/v1".into());
    let repeated = json!({"role":"user","content":"exact repeated evidence"});
    let body = super::super::provider_protocols::anthropic_request_body(&cfg,
        &[repeated.clone(), repeated.clone()], &tools(), "auto").unwrap();
    let snapshot = PreparedModelRequest {config:cfg, body, completed_response:vec![], started_at:Instant::now()};
    let usage = LlmUsage {input_tokens: Some(5000), cached_input_tokens: Some(0), cache_write_input_tokens: Some(0), ..Default::default()};
    assert_eq!(snapshot.fork(&[repeated], "summary", 1024, 128000, &usage, Default::default()).err(), Some("archive_range_not_unique"));
    assert_eq!(snapshot.fork(&[json!({"role":"user","content":"exact repeated"})], "summary", 1024, 128000, &usage, Default::default()).err(), Some("archive_not_in_snapshot"));
}

fn fixture_snapshot() -> (PreparedModelRequest, Vec<Value>, LlmUsage) {
    let cfg = config("openai", "https://api.openai.com/v1".into());
    let history = history();
    let body =
        super::super::streaming::openai_request_body(&cfg, &history, &tools(), "auto").unwrap();
    (
        PreparedModelRequest {
            config: cfg,
            body,
            completed_response: Vec::new(),
            started_at: Instant::now(),
        },
        history[1..2].to_vec(),
        LlmUsage {
            input_tokens: Some(12000),
            cached_input_tokens: Some(11000),
            ..Default::default()
        },
    )
}

#[test]
fn cache_identity_tracks_sent_tools_system_policy_and_actual_route_not_history_growth() {
    let (mut request, _, _) = fixture_snapshot();
    let original = request.prompt_cache_identity();
    request.body["messages"].as_array_mut().unwrap().push(json!({"role":"user","content":"follow up"}));
    assert_eq!(request.prompt_cache_identity(), original, "appending history preserves the static prefix");
    request.body["tools"].as_array_mut().unwrap().push(json!({"type":"function","function":{"name":"discovered_tool","parameters":{"type":"object"}}}));
    let discovered = request.prompt_cache_identity();
    assert_ne!(discovered.tool_manifest_sha256, original.tool_manifest_sha256);
    assert_ne!(discovered.stable_prefix_sha256, original.stable_prefix_sha256);
    request.body["tool_choice"] = json!("none");
    assert_ne!(request.prompt_cache_identity().stable_prefix_sha256, discovered.stable_prefix_sha256);
    let policy = request.prompt_cache_identity();
    request.config.endpoint = "https://another-host.invalid/v1".into();
    assert_ne!(request.prompt_cache_identity().stable_prefix_sha256, policy.stable_prefix_sha256);
    let route = request.prompt_cache_identity();
    request.body["messages"][0] = json!({"role":"system","content":"Changed actual system instructions"});
    assert_ne!(request.prompt_cache_identity().stable_prefix_sha256, route.stable_prefix_sha256);
}

#[test]
fn durable_fallback_restores_only_from_current_admitted_route_and_credentials() {
    let mut primary = config("openai", "http://primary.invalid/v1".into());
    primary.fallbacks.push(super::super::contracts::LlmProviderTarget {
        provider:Some("openai".into()), endpoint:"http://fallback.invalid/v1".into(),
        api_key:Some("fallback-private-key".into()), model:Some("fallback-model".into()),
    });
    let fallback = super::super::contracts::admitted_provider_targets(&primary).remove(1);
    let body = super::super::streaming::openai_request_body(&fallback, &history(), &tools(), "auto").unwrap();
    let snapshot = PreparedModelRequest {config:fallback, body:body.clone(), completed_response:vec![], started_at:Instant::now()};
    let stored = snapshot.durable_value();
    assert!(!stored.to_string().contains("fallback-private-key"));
    let restored = PreparedModelRequest::restore(&stored, &primary).unwrap();
    assert_eq!(restored.body, body);
    assert_eq!(restored.config.model, "fallback-model");
    assert_eq!(restored.config.api_key, "fallback-private-key");
    let mut changed = primary.clone();
    changed.fallbacks[0].api_key = Some("rotated-key".into());
    assert!(PreparedModelRequest::restore(&stored, &changed).is_none());
    primary.fallbacks.clear();
    assert!(PreparedModelRequest::restore(&stored, &primary).is_none());
}

#[test]
fn compaction_fork_matches_tool_results_with_host_artifact_metadata() {
    let cfg = config("openai", "http://127.0.0.1:1/v1".into());
    let archive = vec![
        json!({"role":"assistant","content":"","tool_calls":[{"id":"edit-1","type":"function","function":{"name":"edit_file","arguments":"{}"}}]}),
        json!({"role":"tool","tool_call_id":"edit-1","name":"edit_file","content":"exact edit result",
            "_nola_artifacts":[{"uri":"file:///project/invoices.py","operation":"edit","mime_type":"text/plain"}]}),
    ];
    let mut physical = archive.clone();
    physical[1].as_object_mut().unwrap().remove("_nola_artifacts");
    physical.push(json!({"role":"user","content":"continue"}));
    let body = super::super::streaming::openai_request_body(&cfg, &physical, &tools(), "auto").unwrap();
    let snapshot = PreparedModelRequest { config:cfg, body:body.clone(), completed_response:vec![], started_at:Instant::now() };
    let usage = LlmUsage { input_tokens: Some(12000), cached_input_tokens:Some(11000), ..Default::default() };
    let fork = snapshot.fork(&archive, "summarize", 2048, 128000, &usage, Default::default()).unwrap();
    let mut restored = fork.body.clone();
    restored["messages"] = body["messages"].clone();
    assert_eq!(restored, body, "only append to the actual physical request");
    let mut changed = archive;
    changed[1]["content"] = json!("different edit result");
    assert_eq!(snapshot.fork(&changed, "summarize", 2048, 128000, &usage, Default::default()).err(), Some("archive_not_in_snapshot"));
}

#[test]
fn compaction_fork_preserves_actual_http_request_prefix_for_all_protocols() {
    for provider in ["openai", "openai-responses", "anthropic"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut cfg = config(
            provider,
            format!("http://{}/v1", listener.local_addr().unwrap()),
        );
        cfg.reasoning_selection = Some(if provider == "anthropic" {
            nola_domain::generation::ReasoningSelection::Budget { tokens: 1024 }
        } else {
            nola_domain::generation::ReasoningSelection::Effort {
                value: "high".into(),
            }
        });
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let deadline = Instant::now() + Duration::from_secs(10);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(socket) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing compaction request");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    assert!(!line.is_empty());
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                requests.push(serde_json::from_slice::<Value>(&bytes).unwrap());
                let events = match provider {
                    "anthropic" => vec![
                        json!({"type":"message_start","message":{"usage":{"input_tokens":1000,"cache_read_input_tokens":11000,"cache_creation_input_tokens":0}}}),
                        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"summary"}}),
                        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}),
                        json!({"type":"message_stop"}),
                    ],
                    "openai-responses" => vec![json!({"type":"response.completed","response":{
                        "object":"response","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"summary"}]}],
                        "usage":{"input_tokens":12000,"output_tokens":5,"input_tokens_details":{"cached_tokens":11000}}}})],
                    _ => vec![
                        json!({"choices":[{"delta":{"content":"summary"},"finish_reason":"stop"}],
                        "usage":{"prompt_tokens":12000,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":11000}}}),
                    ],
                };
                let body = events
                    .iter()
                    .map(|event| format!("data: {event}\n\n"))
                    .collect::<String>();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        let history = history();
        let choice =
            super::super::streaming::stream_chat_checked(&cfg, &history, &tools(), "auto", |_| {
                Ok(())
            })
            .unwrap();
        let persisted = choice.request_snapshot.as_ref().unwrap().durable_value();
        let snapshot = PreparedModelRequest::restore(&persisted, &cfg).unwrap();
        let fork = snapshot
            .fork(
                &history[1..2],
                "Summarize the archive accurately.",
                2048,
                128000,
                &choice.usage,
                Default::default(),
            )
            .unwrap_or_else(|reason| panic!("{provider}: {reason}"));
        assert!(valid_compaction_summary(&fork.stream(|_| Ok(())).unwrap()));
        let requests = server.join().unwrap();
        let key = if provider == "openai-responses" {
            "input"
        } else {
            "messages"
        };
        let mut normalized_fork = requests[1].clone();
        let appended = normalized_fork[key].as_array_mut().unwrap().pop().unwrap();
        assert!(appended["content"].as_str().unwrap()
            .contains(nola_core::agent::compaction_planner::COMPACTION_EVIDENCE_CONTRACT));
        assert!(appended["content"]
            .as_str()
            .unwrap()
            .contains("Do not execute any task or call any tool"));
        assert!(appended["content"]
            .as_str()
            .unwrap()
            .contains("Entries after this range remain verbatim"));
        let budget = if provider == "openai-responses" {
            "max_output_tokens"
        } else if provider == "anthropic" {
            "max_tokens"
        } else {
            super::super::streaming::output_token_parameter_name(&cfg)
        };
        assert_eq!(normalized_fork[budget], 8192);
        normalized_fork[budget] = requests[0][budget].clone();
        assert_eq!(
            serde_json::to_vec(&normalized_fork).unwrap(),
            serde_json::to_vec(&requests[0]).unwrap(),
            "{provider}"
        );
        if provider == "openai-responses" {
            assert_eq!(requests[1]["prompt_cache_key"], "stable-session-key");
        }
        assert!(!format!("{snapshot:?}").contains("private-fixture"));
    }
}

#[test]
fn compaction_fork_rejects_cold_oversized_unaligned_but_accepts_unknown_cache_usage() {
    let (mut snapshot, archive, mut usage) = fixture_snapshot();
    assert!(snapshot
        .fork(
            &archive,
            "summary",
            2048,
            128000,
            &usage,
            Default::default()
        )
        .is_ok());
    snapshot.started_at = Instant::now() - Duration::from_secs(241);
    assert_eq!(
        snapshot
            .fork(
                &archive,
                "summary",
                2048,
                128000,
                &usage,
                Default::default()
            )
            .err(),
        Some("cache_age_limit")
    );
    snapshot.started_at = Instant::now();
    assert_eq!(
        snapshot
            .fork(&archive, "summary", 2048, 14000, &usage, Default::default())
            .err(),
        Some("window_reserve_insufficient")
    );
    assert_eq!(
        snapshot
            .fork(
                &[json!({"role":"user","content":"not in history"})],
                "summary",
                2048,
                128000,
                &usage,
                Default::default()
            )
            .err(),
        Some("archive_not_in_snapshot")
    );
    usage.cached_input_tokens = None;
    assert!(
        snapshot
            .fork(
                &archive,
                "summary",
                2048,
                128000,
                &usage,
                Default::default()
            )
            .is_ok()
    );
}

#[test]
fn compaction_append_estimate_admits_warm_prefix_without_borrowing_output_reserve() {
    let (snapshot, archive, usage) = fixture_snapshot();
    let instructions = "Preserve constraints and unresolved work. ".repeat(80);
    let roomy = snapshot.fork(&archive, &instructions, 2048, 128000, &usage, Default::default()).unwrap();
    let append = roomy.body["messages"].as_array().unwrap().last().unwrap();
    let window = usage.input_tokens.unwrap() + snapshot.config.max_output_tokens as usize
        + estimated_extension_tokens(append) + 1024;
    let admitted = snapshot.fork(&archive, &instructions, 2048, window, &usage, Default::default()).unwrap();
    assert_eq!(admitted.config.max_output_tokens, snapshot.config.max_output_tokens);
    assert_eq!(admitted.body, roomy.body);
    assert_eq!(snapshot.fork(&archive, &instructions, 2048, window - 1, &usage, Default::default()).err(),
        Some("window_reserve_insufficient"));
    assert!(estimated_extension_tokens(&json!("中文约束".repeat(100))) >= 600);
}

#[test]
fn compaction_rejects_tool_calls_truncation_and_empty_summaries() {
    let mut choice = StreamedChoice {
        content: "summary".into(),
        finish_reason: "stop".into(),
        ..Default::default()
    };
    assert!(valid_compaction_summary(&choice));
    for reason in [
        "length",
        "max_tokens",
        "max_output_bytes",
        "tool_calls",
        "content_filter",
        "",
    ] {
        choice.finish_reason = reason.into();
        assert!(!valid_compaction_summary(&choice));
    }
    choice.finish_reason = "stop".into();
    choice.tool_calls.push(super::super::contracts::ToolCall {
        id: "call".into(),
        name: "write_file".into(),
        arguments: "{}".into(),
    });
    assert!(!valid_compaction_summary(&choice));
    choice.tool_calls.clear();
    choice.content = " \n".into();
    assert!(!valid_compaction_summary(&choice));
}

#[test]
fn compaction_fork_keeps_unselected_anchor_between_source_ranges() {
    let (snapshot, _, usage) = fixture_snapshot();
    let history = history();
    let source = vec![history[1].clone(), history[3].clone()];
    let fork = snapshot.fork(&source, "Summarize selected evidence only", 2048, 128000,
        &usage, Default::default()).unwrap();
    let messages = fork.body["messages"].as_array().unwrap();
    assert_eq!(&messages[..messages.len()-1], snapshot.body["messages"].as_array().unwrap().as_slice());
    let instruction = messages.last().unwrap()["content"].as_str().unwrap();
    assert!(instruction.contains("2 through 2, 4 through 4"));
    assert!(instruction.contains("including entries between ranges"));
}

#[test]
fn compaction_configuration_identity_excludes_cancellation_but_not_model_or_credentials() {
    let (snapshot, _, _) = fixture_snapshot();
    let identity = request_configuration_identity(&snapshot.config);
    let mut cfg = snapshot.config.clone();
    cfg.cancellation = Default::default();
    assert_eq!(identity, request_configuration_identity(&cfg));
    cfg.api_key = "other-secret".into();
    assert_ne!(identity, request_configuration_identity(&cfg));
    cfg = snapshot.config.clone();
    cfg.model = "other-model".into();
    assert_ne!(identity, request_configuration_identity(&cfg));
}

#[test]
fn compaction_includes_completed_response_after_restart_without_rewriting_prefix() {
    for provider in ["openai", "openai-responses", "anthropic"] {
        let (mut snapshot, _, mut usage) = fixture_snapshot();
        if provider == "anthropic" { usage.cache_write_input_tokens = Some(0); }
        snapshot.config.provider = Some(provider.into());
        let messages = history();
        snapshot.body = match provider {
            "anthropic" => super::super::provider_protocols::anthropic_request_body(
                &snapshot.config, &messages, &tools(), "auto"),
            "openai-responses" => super::super::responses::responses_body(
                &snapshot.config, &messages, &tools(), "auto"),
            _ => super::super::streaming::openai_request_body(
                &snapshot.config, &messages, &tools(), "auto"),
        }.unwrap();
        snapshot.record_completed_response(&StreamedChoice {
            content: "Final answer after the last request".into(),
            response_items: if provider == "openai-responses" { vec![
                json!({"type":"reasoning","id":"rs_fixture","encrypted_content":"opaque-fixture"}),
                json!({"type":"message","role":"assistant","content":[
                    {"type":"output_text","text":"Final answer after the last request"}]})
            ] } else { vec![] },
            finish_reason: "stop".into(), ..Default::default()
        });
        let restored = PreparedModelRequest::restore(&snapshot.durable_value(), &snapshot.config).unwrap();
        let mut source = messages[1..].to_vec();
        source.extend(restored.completed_response.iter().cloned());
        if provider == "anthropic" {
            let mut unknown_usage = usage.clone();
            unknown_usage.cache_write_input_tokens = None;
            assert_eq!(restored.fork(&source, "summarize", 2048, 128000,
                &unknown_usage, Default::default()).err(), Some("input_usage_unknown"));
        }
        let fork = restored.fork(&source, "summarize", 2048, 128000, &usage, Default::default())
            .unwrap_or_else(|error| panic!("{provider}: {error}"));
        let key = if provider == "openai-responses" { "input" } else { "messages" };
        let original = snapshot.body[key].as_array().unwrap();
        let extended = fork.body[key].as_array().unwrap();
        assert_eq!(&extended[..original.len()], original.as_slice(), "{provider}");
        assert_eq!(extended.len(), original.len() + if provider == "openai-responses" {3} else {2}, "{provider}");
        for field in ["tools", "system", "instructions", "tool_choice", "thinking", "reasoning"] {
            assert_eq!(snapshot.body[field], fork.body[field], "{provider}: {field}");
        }
    }
}

#[test]
fn compaction_matches_repeated_answers_only_when_the_whole_selection_is_unique() {
    let (mut snapshot, _, usage) = fixture_snapshot();
    let source = vec![json!({"role":"user","content":"first"}),
        json!({"role":"assistant","content":"ACK"}),
        json!({"role":"user","content":"second"}),
        json!({"role":"assistant","content":"ACK"})];
    snapshot.body["messages"] = json!(source);
    assert!(snapshot.fork(&source, "summarize", 2048, 128000, &usage, Default::default()).is_ok());
    assert_eq!(snapshot.fork(&source[1..2], "summarize", 2048, 128000,
        &usage, Default::default()).err(), Some("archive_range_not_unique"));
}

#[test]
fn history_cache_marker_alignment_preserves_text_and_ignores_only_hints() {
    let marked=json!({"role":"user","content":[{"type":"input_text","text":"history","prompt_cache_breakpoint":{"mode":"explicit"}}]});
    assert_eq!(without_cache_markers(&marked),json!({"role":"user","content":"history"}));
    assert!(marked["content"][0].get("prompt_cache_breakpoint").is_some());
}
