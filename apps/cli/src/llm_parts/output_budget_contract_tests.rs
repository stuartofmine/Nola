// SPDX-License-Identifier: Apache-2.0
//! Provider wire output budgets must equal the frozen admitted preference.
use super::contracts::{effective_max_output_tokens, LlmConfig};
use super::streaming::stream_chat_checked;
use nola_domain::generation::ReasoningSelection;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

fn configuration(provider: &str, endpoint: String, tokens: u64) -> LlmConfig {
    LlmConfig {
        siwc: None, native_model_route: None,
        provider: Some(provider.into()),
        fallbacks: vec![],
        endpoint,
        api_key: String::new(),
        model: "budget-fixture".into(),
        temperature: 0.7,
        top_p: 0.9,
        max_output_tokens: tokens,
        compaction_output_budget: None,
        chat_output_token_field: Default::default(),
        max_retries: 0,
        parallel_tool_calls: None,
        thinking_enabled: true, reasoning_summary_supported: None,
        reasoning_selection: None,
        sampling_override: false,
        generation_options: Default::default(),
        cancellation: Default::default(),
        openai_prompt_cache: false,
        openai_prompt_cache_key: None,
        openai_prompt_cache_key_prefix: None,
        openai_prompt_cache_retention: None,
        openai_prompt_cache_history_breakpoint: false,
        openai_prompt_cache_history_breakpoint_endpoints: None,
    }
}

#[test]
fn compaction_disables_gateway_reasoning_without_changing_automatic_turns() {
    use super::provider_protocols::apply_thinking_policy;
    for (provider, endpoint, model) in [
        ("openrouter", "http://127.0.0.1:1234/v1", "deepseek-r1"),
        ("openai-compatible", "https://openrouter.ai/api/v1", "deepseek-r1"),
    ] {
        let mut cfg = configuration(provider, endpoint.into(), 16000);
        cfg.model = model.into();
        cfg.thinking_enabled = false;
        let mut normal = json!({"messages": []});
        apply_thinking_policy(&cfg, &mut normal);
        assert!(normal.get("reasoning").is_none());
        cfg = cfg.for_compaction().unwrap();
        let mut cold = json!({"messages": []});
        apply_thinking_policy(&cfg, &mut cold);
        assert_eq!(cold["reasoning"], json!({"enabled":false}));
        assert!(cold.get("thinking").is_none(), "gateway must take precedence over model vendor");
    }
}

#[test]
fn compaction_openrouter_disable_reaches_the_physical_http_body() {
    let (_, body) = captured_configured_wire_request("openrouter", 16000, None, "", |cfg| {
        *cfg = cfg.for_compaction().unwrap();
        cfg.thinking_enabled = false;
    });
    assert_eq!(body["reasoning"], json!({"enabled":false}));
    assert_eq!(body["max_tokens"], 16000);
    assert!(body.get("thinking").is_none());
}

#[test]
fn durable_compaction_snapshot_restores_prefix_without_serializing_credentials() {
    let mut config = configuration("openai", "https://fixture.invalid/v1".into(), 8192);
    config.api_key = "fixture-secret-do-not-persist".into();
    let body = json!({"model":"budget-fixture", "messages":[{"role":"user","content":"preserve this exact prefix"}],
        "tools":[], "tool_choice":"auto"});
    let result = super::contracts::retain_request(Ok(Default::default()), &config, body.clone(), Instant::now()).unwrap();
    let persisted = result.request_snapshot.unwrap().durable_value();
    assert!(!persisted.to_string().contains(&config.api_key));
    let restored = super::PreparedModelRequest::restore(&persisted, &config).unwrap();
    assert_eq!(restored.durable_value()["body"], body);
    // Core may have clamped the physical budget below the Host preference.
    // The enclosing session witness guards preference changes; restoration
    // must reconstruct the accepted physical settings, not invent new ones.
    config.max_output_tokens = 16384;
    let restored = super::PreparedModelRequest::restore(&persisted, &config).unwrap();
    assert_eq!(restored.config().max_output_tokens, 8192);
    config.model = "another-model".into();
    assert!(super::PreparedModelRequest::restore(&persisted, &config).is_none());
}

fn captured_request(provider: &str, tokens: u64, reasoning: Option<ReasoningSelection>) -> Value {
    captured_wire_request(provider, tokens, reasoning, "").1
}

fn captured_wire_request(
    provider: &str,
    tokens: u64,
    reasoning: Option<ReasoningSelection>,
    suffix: &str,
) -> (String, Value) {
    captured_configured_wire_request(provider, tokens, reasoning, suffix, |_| {})
}

fn captured_configured_wire_request(
    provider: &str,
    tokens: u64,
    reasoning: Option<ReasoningSelection>,
    suffix: &str,
    configure: impl FnOnce(&mut LlmConfig),
) -> (String, Value) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}/v1{suffix}", listener.local_addr().unwrap());
    let response_events = match provider.trim().to_ascii_lowercase().as_str() {
        "anthropic" => vec![
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
            json!({"type":"message_stop"}),
        ],
        "openai-responses" => vec![json!({"type":"response.completed","response":{
            "object":"response","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"ok"}]}]
        }})],
        _ => vec![json!({"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]})],
    };
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "provider did not issue a request"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept fixture request: {error}"),
            }
        };
        // Accepted sockets inherit O_NONBLOCK on macOS. Acceptance is bounded
        // above; request reads use the explicit timeout below, not readiness races.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let request_line = line.clone();
        let mut length = None;
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
        }
        let mut bytes = vec![0; length.expect("JSON request content length")];
        reader.read_exact(&mut bytes).unwrap();
        let request = serde_json::from_slice::<Value>(&bytes).unwrap();
        let body = response_events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        (request_line, request)
    });
    let mut cfg = configuration(provider, endpoint, tokens);
    cfg.reasoning_selection = reasoning;
    configure(&mut cfg);
    let admitted_tokens = cfg.max_output_tokens;
    let result = stream_chat_checked(
        &cfg,
        &[json!({"role":"user","content":"fixture"})],
        &[],
        "auto",
        |_| Ok(()),
    );
    let request = server.join().unwrap();
    assert_eq!(result.unwrap().content, "ok");
    assert_eq!(cfg.max_output_tokens, admitted_tokens);
    request
}

#[test]
fn resolved_seed_is_exact_or_absent_on_actual_http_wire() {
    for seed in [
        None,
        Some(42),
        Some(0),
        Some(-1),
        Some(9_007_199_254_740_993),
        Some(i64::MIN),
        Some(i64::MAX),
    ] {
        let (_, body) =
            captured_configured_wire_request("openai-compatible", 256, None, "", |cfg| {
                cfg.generation_options.seed = seed
            });
        match seed {
            Some(value) => assert_eq!(body["seed"].as_i64(), Some(value)),
            None => assert!(
                body.get("seed").is_none(),
                "Clear must omit seed, not send null or zero"
            ),
        }
    }
    let (_, body) = captured_configured_wire_request("openai-responses", 256, None, "", |cfg| {
        cfg.generation_options.seed = None
    });
    assert!(body.get("seed").is_none());
}

#[test]
fn explicit_normalized_protocol_drives_real_wire_path_and_output_field() {
    for (provider, path, field) in [
        (" OPENAI-RESPONSES ", "/responses", "max_output_tokens"),
        (" ANTHROPIC ", "/messages", "max_tokens"),
        (" OpenAI-Compatible ", "/chat/completions", "max_tokens"),
    ] {
        let (line, body) = captured_wire_request(provider, 256, None, "");
        assert!(
            line.split_whitespace().nth(1).unwrap().ends_with(path),
            "{line}"
        );
        assert_eq!(body[field], 256);
        for other in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
            if other != field {
                assert!(body.get(other).is_none(), "{body}");
            }
        }
    }
}

#[test]
fn endpoint_query_cannot_spoof_anthropic_wire_dispatch() {
    let (line, body) = captured_wire_request("", 256, None, "?redirect=api.anthropic.com");
    assert_eq!(
        line.split_whitespace().nth(1).unwrap(),
        "/v1/chat/completions?redirect=api.anthropic.com"
    );
    assert!(body.get("messages").is_some());
    assert_eq!(body["max_tokens"], 256);
}

#[test]
fn all_protocols_preserve_exact_query_after_path_normalization_on_real_http() {
    for (provider, suffix, target) in [
        (
            "openai",
            "/?key=a%2Fb&key=c+z",
            "/v1/chat/completions?key=a%2Fb&key=c+z",
        ),
        (
            "openai",
            "/chat/completions/?key=a%2Fb",
            "/v1/chat/completions?key=a%2Fb",
        ),
        ("openai-responses", "/?key=a%2Fb", "/v1/responses?key=a%2Fb"),
        (
            "openai-responses",
            "/responses/?key=a%2Fb",
            "/v1/responses?key=a%2Fb",
        ),
        (
            "openai-responses",
            "/chat/completions/?key=a%2Fb",
            "/v1/responses?key=a%2Fb",
        ),
        ("anthropic", "/?key=a%2Fb", "/v1/messages?key=a%2Fb"),
        (
            "anthropic",
            "/messages/?key=a%2Fb",
            "/v1/messages?key=a%2Fb",
        ),
    ] {
        let (line, _) = captured_wire_request(provider, 256, None, suffix);
        assert_eq!(line.split_whitespace().nth(1).unwrap(), target);
    }
}

#[test]
fn endpoint_validation_and_base_path_rules_are_protocol_explicit() {
    use super::transport::normalized_model_endpoint;
    use nola_domain::generation::ModelProtocol::*;
    for (protocol, base, expected) in [
        (ChatCompletions, "", "/chat/completions"),
        (Responses, "/", "/responses"),
        (Anthropic, "", "/v1/messages"),
        (Anthropic, "/gateway/", "/gateway/v1/messages"),
        (Anthropic, "/gateway/messages/", "/gateway/messages"),
        (
            ChatCompletions,
            "/prefix%2Ftenant/v1/",
            "/prefix%2Ftenant/v1/chat/completions",
        ),
    ] {
        let result =
            normalized_model_endpoint(&format!("https://example.invalid{base}?x=1"), protocol)
                .unwrap();
        assert_eq!(result.path(), expected);
        assert_eq!(result.query(), Some("x=1"));
    }
    for protocol in [ChatCompletions, Responses, Anthropic] {
        for endpoint in [
            "file:///tmp/secret",
            "https://user:secret@example.invalid/v1",
            "https://example.invalid/v1?key=secret#fragment",
            "secret",
        ] {
            let error = normalized_model_endpoint(endpoint, protocol).unwrap_err();
            assert!(!error.contains("secret"));
        }
    }
}

#[test]
fn transport_failure_does_not_expose_credential_query_in_diagnostics() {
    use super::transport::{send_request_with_cancellation, SendRequestResult};
    // Keep the bound listener alive but never service it, deterministically
    // timing out this local request without external networking or port races.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(100))
        .build()
        .unwrap();
    let request = client.get(format!(
        "http://{}/v1?key=fixture-secret",
        listener.local_addr().unwrap()
    ));
    let SendRequestResult::Error(error) =
        send_request_with_cancellation(request, &Default::default())
    else {
        panic!("unserviced local request must fail");
    };
    assert!(error.url().is_none());
    assert!(!error.to_string().contains("fixture-secret"));
}

#[test]
fn all_route_consumers_agree_on_explicit_protocol_over_conflicting_authority() {
    use super::contracts::resolved_model_protocol;
    use super::streaming::{output_token_parameter_name, provider_continuation_namespace};
    use nola_domain::generation::ModelProtocol;
    for (provider, protocol, field, namespace) in [
        (
            " OPENAI-RESPONSES ",
            ModelProtocol::Responses,
            "max_output_tokens",
            "openai.responses",
        ),
        (
            " Chat_Completions ",
            ModelProtocol::ChatCompletions,
            "max_tokens",
            "openai-compatible.chat-completions",
        ),
        (
            " ANTHROPIC ",
            ModelProtocol::Anthropic,
            "max_tokens",
            "anthropic.messages",
        ),
    ] {
        let mut cfg = configuration(provider, "https://api.anthropic.com/v1".into(), 256);
        for model in ["gpt-test", "claude-test", "qwen-test"] {
            cfg.model = model.into();
            assert_eq!(
                resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint),
                protocol
            );
            assert_eq!(output_token_parameter_name(&cfg), field);
            assert_eq!(provider_continuation_namespace(&cfg), namespace);
        }
    }
    for endpoint in [
        "https://other.invalid/api.anthropic.com",
        "https://api.anthropic.com.evil.invalid/v1",
        "https://api.anthropic.com@other.invalid/v1",
    ] {
        assert_eq!(
            resolved_model_protocol(None, endpoint),
            ModelProtocol::ChatCompletions
        );
    }
}

#[test]
fn admitted_output_limits_are_exact_and_invalid_limits_are_not_defaulted() {
    let mut cfg = configuration("openai", "http://127.0.0.1:1/v1".into(), 64);
    for value in [64, 32768, 32769, 64000, 1_000_000] {
        cfg.max_output_tokens = value;
        assert_eq!(effective_max_output_tokens(&cfg).unwrap(), value);
    }
    for value in [0, 1, 63, 1_000_001, u64::MAX] {
        cfg.max_output_tokens = value;
        assert!(effective_max_output_tokens(&cfg).is_err());
    }
}

#[test]
fn all_provider_requests_preserve_small_and_large_admitted_output_limits() {
    for provider in ["openai", "openai-responses", "anthropic"] {
        for tokens in [64, 64000] {
            let request = captured_request(provider, tokens, None);
            let key = if provider == "openai-responses" {
                "max_output_tokens"
            } else {
                "max_tokens"
            };
            assert_eq!(
                request[key], tokens,
                "{provider} must not apply a second budget policy"
            );
        }
    }
}

#[test]
fn compaction_preserves_configured_output_without_an_internal_cap() {
    for configured in [64, 512, 4096, 32000, 1_000_000] {
        let cfg = configuration("openai", "http://127.0.0.1:1/v1".into(), configured);
        {
            let compact = cfg.for_compaction().unwrap();
            let tokens = effective_max_output_tokens(&compact).unwrap();
            assert_eq!(tokens, configured);
            let mut changed = compact.clone();
            changed.max_output_tokens += 1;
            assert!(effective_max_output_tokens(&changed).is_err());
        }
    }
    for invalid in [0, 1, 63, 1_000_001, u64::MAX] {
        let cfg = configuration("openai", "http://127.0.0.1:1/v1".into(), invalid);
        assert!(effective_max_output_tokens(&cfg).is_err());
        assert!(cfg.for_compaction().is_err());
    }
}

#[test]
fn all_provider_wires_keep_the_configured_compaction_output_allowance() {
    for provider in ["openai", "openai-responses", "anthropic"] {
        for target in [4096, 16000, 32000] {
            let (_, body) = captured_configured_wire_request(provider, target, None, "", |cfg| {
                *cfg = cfg.for_compaction().unwrap();
            });
            let field = if provider == "openai-responses" { "max_output_tokens" } else { "max_tokens" };
            assert_eq!(body[field].as_u64(), Some(target), "{provider}: {body}");
        }
    }
}

#[test]
fn anthropic_large_thinking_budget_uses_the_same_admitted_output_limit() {
    let request = captured_request(
        "anthropic",
        64000,
        Some(ReasoningSelection::Budget { tokens: 40000 }),
    );
    assert_eq!(request["max_tokens"], 64000);
    assert_eq!(request["thinking"]["budget_tokens"], 40000);
}

#[test]
fn zero_output_limit_is_rejected_before_any_provider_request() {
    for provider in ["openai", "openai-responses", "anthropic"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let cfg = configuration(
            provider,
            format!("http://{}/v1", listener.local_addr().unwrap()),
            0,
        );
        assert!(stream_chat_checked(
            &cfg,
            &[json!({"role":"user","content":"fixture"})],
            &[],
            "auto",
            |_| Ok(())
        )
        .is_err());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
