// SPDX-License-Identifier: Apache-2.0
//! Cross-provider contract tests live with the streaming driver whose private
//! text framing and delivery state they exercise.

use super::super::contracts::LlmUsage;
use super::super::diagnostics::{
    format_local_debug_timestamp, model_request_debug_payload, model_response_debug_payload,
};
use super::super::provider_protocols::{
    anthropic_request_body, apply_thinking_policy, attach_anthropic_cache_breakpoint,
    detect_chat_provider, non_stream_required_tools_enabled, normalize_openai_compatible_messages,
    parse_anthropic_stream_event, ChatProvider, OpenAiFlavor,
};
use super::super::stream_state::should_stop_repetitive_output;
use super::super::transport::resolve_dns_fallbacks_for_host;
use super::{
    append_openai_tool_delta, apply_main_turn_prompt_cache_identity, normalize_tool_arguments_text,
    normalized_tool_arguments_value, parse_text_tool_calls, tool_argument_fragment_text,
    tool_arguments_text, LlmConfig, LlmEvent, StreamedChoice, TextToolProtocolBuffer, ToolCall,
    ToolSlot,
};
use std::collections::BTreeMap;
use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;

use serde_json::json;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn debug_timestamp_zero_pads_milliseconds() {
    assert_eq!(format_local_debug_timestamp(123, 5), "123.005");
    assert_eq!(format_local_debug_timestamp(123, 95), "123.095");
    assert_eq!(format_local_debug_timestamp(123, 995), "123.995");
}

#[test]
fn parses_dsml_tool_calls_as_native_tool_calls() {
    let content = r#"
<｜｜DSML｜｜tool_calls>
<｜｜DSML｜｜invoke name="skill_context">
<｜｜DSML｜｜parameter name="name" string="true">word</｜｜DSML｜｜parameter>
<｜｜DSML｜｜parameter name="include_files" string="false">false</｜｜DSML｜｜parameter>
<｜｜DSML｜｜parameter name="limit" string="false">3</｜｜DSML｜｜parameter>
</｜｜DSML｜｜invoke>
</｜｜DSML｜｜tool_calls>
"#;

    let calls = parse_text_tool_calls(content).expect("DSML tool call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "skill_context");
    let arguments: serde_json::Value =
        serde_json::from_str(&calls[0].arguments).expect("arguments JSON");
    assert_eq!(arguments["name"], "word");
    assert_eq!(arguments["include_files"], false);
    assert_eq!(arguments["limit"], 3);
}

#[test]
fn parses_deepseek_dsml_tool_tag_as_native_tool_call() {
    let content = r#"
<｜｜DSML｜｜tool_calls>
<｜｜DSML｜｜tool name="write_file">
<｜｜DSML｜｜parameter name="path" string="true">report.json</｜｜DSML｜｜parameter>
<｜｜DSML｜｜parameter name="json_content" string="false">{"title":"AI"}</｜｜DSML｜｜parameter>
</｜｜DSML｜｜tool>
</｜｜DSML｜｜tool_calls>
"#;

    let calls = parse_text_tool_calls(content).expect("DeepSeek DSML tool call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "write_file");
    let arguments: serde_json::Value =
        serde_json::from_str(&calls[0].arguments).expect("arguments JSON");
    assert_eq!(arguments["path"], "report.json");
    assert_eq!(arguments["json_content"]["title"], "AI");
}

#[test]
fn preserves_object_shaped_openai_tool_arguments() {
    let arguments = json!({
        "path": "outputs/report.html",
        "content": "<h1>Report</h1>"
    });

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &tool_arguments_text(Some(&arguments)).expect("serialized arguments")
        )
        .expect("valid JSON"),
        arguments
    );
    assert_eq!(
        tool_arguments_text(Some(&json!(" {\"path\":\"report.md\"} "))),
        Some("{\"path\":\"report.md\"}".to_string())
    );
    assert_eq!(
        tool_arguments_text(Some(&json!(
            "\"{\\\"path\\\":\\\"report.md\\\",\\\"content\\\":\\\"ready\\\"}\""
        ))),
        Some("{\"content\":\"ready\",\"path\":\"report.md\"}".to_string())
    );
    assert_eq!(tool_arguments_text(Some(&json!(""))), None);
    assert_eq!(tool_arguments_text(Some(&serde_json::Value::Null)), None);
}

#[test]
fn normalizes_double_encoded_stream_tool_arguments_after_assembly() {
    let encoded = "\"{\\\"path\\\":\\\"transformer.py\\\",\\\"content\\\":\\\"print(1)\\\"}\"";
    assert_eq!(
        normalize_tool_arguments_text(encoded),
        "{\"content\":\"print(1)\",\"path\":\"transformer.py\"}"
    );
}

#[test]
fn converts_double_encoded_arguments_to_an_object_before_core_submission() {
    let encoded = "\"{\\\"path\\\":\\\"transformer.py\\\",\\\"content\\\":\\\"print(1)\\\"}\"";
    assert_eq!(
        normalized_tool_arguments_value(encoded),
        json!({"path": "transformer.py", "content": "print(1)"})
    );
}

#[test]
fn openai_stream_tool_arguments_preserve_whitespace_only_deltas() {
    let mut slots = BTreeMap::<usize, ToolSlot>::new();
    for delta in [
        json!({
            "index": 0,
            "id": "call_space",
            "function": {
                "name": "bash",
                "arguments": "{\"command\":\"make"
            }
        }),
        json!({"index": 0, "function": {"arguments": " "}}),
        json!({"index": 0, "function": {"arguments": "2>&1\"}"}}),
    ] {
        append_openai_tool_delta(&mut slots, &delta).expect("valid tool delta");
    }

    assert_eq!(
        slots.get(&0).map(|slot| slot.arguments.as_str()),
        Some("{\"command\":\"make 2>&1\"}")
    );
    assert_eq!(
        tool_argument_fragment_text(Some(&json!("\t"))),
        Some("\t".to_string())
    );
}

#[test]
fn dsml_stream_is_withheld_from_text_and_returns_tool_call() {
    let mut choice = StreamedChoice::default();
    let mut buffer = TextToolProtocolBuffer::default();
    let mut visible = Vec::new();
    for chunk in [
        "<",
        "｜｜DSML｜｜tool_calls>\n<｜｜DSML｜｜invoke name=\"skill_context\">",
        "<｜｜DSML｜｜parameter name=\"name\" string=\"true\">word</｜｜DSML｜｜parameter>",
        "</｜｜DSML｜｜invoke>\n</｜｜DSML｜｜tool_calls>",
    ] {
        assert!(buffer
            .push(&mut choice, chunk, &mut |event| {
                if let LlmEvent::TextDelta(text) = event {
                    visible.push(text);
                }
                Ok(())
            })
            .unwrap());
    }
    let calls = buffer
        .finish(&mut choice, &mut |event| {
            if let LlmEvent::TextDelta(text) = event {
                visible.push(text);
            }
            Ok(())
        })
        .expect("admitted")
        .expect("tool calls");

    assert!(visible.is_empty());
    assert!(choice.content.is_empty());
    assert_eq!(calls[0].name, "skill_context");
}

#[test]
fn malformed_dsml_stream_is_never_replayed_as_visible_text() {
    let mut choice = StreamedChoice::default();
    let mut buffer = TextToolProtocolBuffer::default();
    let mut visible = Vec::new();
    assert!(buffer
        .push(
            &mut choice,
            "<｜｜DSML｜｜tool_calls><｜｜DSML｜｜tool name=\"write_file\">broken",
            &mut |event| {
                if let LlmEvent::TextDelta(text) = event {
                    visible.push(text);
                }
                Ok(())
            },
        )
        .unwrap());

    assert!(buffer
        .finish(&mut choice, &mut |_| Ok(()))
        .unwrap()
        .is_none());
    assert!(visible.is_empty());
    assert!(choice.content.is_empty());
    assert_eq!(choice.finish_reason, "protocol_error");
}

#[test]
fn misspelled_dsml_envelope_is_control_data_and_never_visible_text() {
    let mut choice = StreamedChoice::default();
    let mut buffer = TextToolProtocolBuffer::default();
    let mut visible = Vec::new();
    for chunk in [
        "我来查看目录。\n\n<｜｜DS",
        "ML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"tool_search\">",
        "<｜｜DSML｜｜ parameter name=\"query\" string=\"true\">list directory</｜｜DSML｜｜ parameter>",
        "</｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>",
    ] {
        assert!(buffer
            .push(&mut choice, chunk, &mut |event| {
                if let LlmEvent::TextDelta(text) = event {
                    visible.push(text);
                }
                Ok(())
            })
            .unwrap());
    }

    assert!(buffer
        .finish(&mut choice, &mut |_| Ok(()))
        .unwrap()
        .is_none());
    assert_eq!(visible.concat(), "我来查看目录。\n\n");
    assert_eq!(choice.content, "我来查看目录。\n\n");
    assert_eq!(choice.finish_reason, "protocol_error");
    assert!(!choice.content.contains("DSML"));
}

#[test]
fn ordinary_angle_bracket_text_is_not_swallowed() {
    let mut choice = StreamedChoice::default();
    let mut buffer = TextToolProtocolBuffer::default();
    let mut visible = Vec::new();
    for chunk in ["<", "section>正文</section>"] {
        assert!(buffer
            .push(&mut choice, chunk, &mut |event| {
                if let LlmEvent::TextDelta(text) = event {
                    visible.push(text);
                }
                Ok(())
            })
            .unwrap());
    }
    assert!(buffer
        .finish(&mut choice, &mut |_| Ok(()))
        .unwrap()
        .is_none());
    assert_eq!(choice.content, "<section>正文</section>");
    assert_eq!(visible.concat(), choice.content);
}

fn without_provider_hint<T>(f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap();
    let previous = env::var_os("NOLA_LLM_PROVIDER");
    let previous_cli = env::var_os("NOLA_CLI_LLM_PROVIDER");
    env::remove_var("NOLA_LLM_PROVIDER");
    env::remove_var("NOLA_CLI_LLM_PROVIDER");
    let out = f();
    if let Some(value) = previous {
        env::set_var("NOLA_LLM_PROVIDER", value);
    }
    if let Some(value) = previous_cli {
        env::set_var("NOLA_CLI_LLM_PROVIDER", value);
    }
    out
}

#[test]
fn openai_compatible_message_normalization_preserves_instruction_order() {
    let messages = vec![
        json!({"role": "user", "content": "ask first"}),
        json!({"role": "system", "content": "late system"}),
        json!({"role": "assistant", "content": "reply"}),
        json!({"role": "system", "content": "early system"}),
    ];
    let normalized = normalize_openai_compatible_messages(&messages);
    assert_eq!(normalized, messages);
}

#[test]
fn openai_continuation_preserves_cached_prefix_and_structured_system_content() {
    let previous = vec![
        json!({"role":"system","content":[{"type":"text","text":"Frozen instructions"}]}),
        json!({"role":"user","content":"Write a long answer"}),
    ];
    let mut continuation = previous.clone();
    continuation.push(json!({"role":"assistant","content":"Retained partial answer"}));
    continuation.push(json!({"role":"system","content":"Continue exactly after the preceding partial answer; do not restart."}));
    let normalized = normalize_openai_compatible_messages(&continuation);
    assert_eq!(normalized, continuation);
    assert_eq!(&normalized[..previous.len()], normalize_openai_compatible_messages(&previous));
    assert_eq!(normalize_openai_compatible_messages(&normalized), normalized);
}

#[test]
fn openai_compatible_message_normalization_preserves_trailing_assistant_reasoning() {
    let messages = vec![
        json!({"role": "system", "content": "system"}),
        json!({"role": "user", "content": "run tool"}),
        json!({
            "role": "assistant",
            "content": "first",
            "reasoning_content": "think one"
        }),
        json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": "think two",
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "read_file",
                    "arguments": "{\"path\":\"README.md\"}"
                }
            }]
        }),
    ];

    let normalized = normalize_openai_compatible_messages(&messages);

    assert_eq!(normalized.len(), 4);
    assert_eq!(normalized[2]["role"], "assistant");
    assert_eq!(normalized[2]["content"], "first");
    assert_eq!(normalized[2]["reasoning_content"], "think one\n\nthink two");
    assert_eq!(
        normalized[2]["tool_calls"][0]["function"]["name"],
        "read_file"
    );
    assert_eq!(normalized[3]["role"], "tool");
    assert_eq!(normalized[3]["tool_call_id"], "call_1");
    assert!(normalized[3]["content"]
        .as_str()
        .unwrap()
        .contains("unavailable"));
}

#[test]
fn openai_message_normalization_repairs_interleaved_parallel_tool_results() {
    let messages = vec![
        json!({"role":"user","content":"run"}),
        json!({
            "role":"assistant",
            "content":"",
            "tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"one","arguments":"{}"}},
                {"id":"call_2","type":"function","function":{"name":"two","arguments":"{}"}},
                {"id":"call_3","type":"function","function":{"name":"three","arguments":"{}"}}
            ]
        }),
        json!({"role":"tool","tool_call_id":"call_1","content":"one"}),
        json!({"role":"user","content":"temporary reflection","_nola_ephemeral":"agent_reflection_hint"}),
        json!({"role":"tool","tool_call_id":"call_3","content":"three"}),
        json!({"role":"tool","tool_call_id":"call_2","content":"two"}),
    ];

    let normalized = normalize_openai_compatible_messages(&messages);

    assert_eq!(normalized[2]["tool_call_id"], "call_1");
    assert_eq!(normalized[3]["tool_call_id"], "call_2");
    assert_eq!(normalized[4]["tool_call_id"], "call_3");
    assert_eq!(normalized[5]["role"], "user");
    assert!(normalized[5].get("_nola_ephemeral").is_none());
}

#[test]
fn openai_message_normalization_fills_missing_tool_results_and_drops_orphans() {
    let messages = vec![
        json!({"role":"tool","tool_call_id":"orphan","content":"orphan"}),
        json!({
            "role":"assistant",
            "content":"",
            "tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"one","arguments":"{}"}},
                {"id":"call_2","type":"function","function":{"name":"two","arguments":"{}"}}
            ]
        }),
        json!({"role":"tool","tool_call_id":"call_1","content":"one"}),
    ];

    let normalized = normalize_openai_compatible_messages(&messages);

    assert_eq!(normalized.len(), 3);
    assert_eq!(normalized[1]["tool_call_id"], "call_1");
    assert_eq!(normalized[2]["tool_call_id"], "call_2");
    assert!(normalized[2]["content"]
        .as_str()
        .unwrap()
        .contains("tool result unavailable"));
}

#[test]
fn parses_dns_fallbacks_from_env() {
    let _guard = ENV_LOCK.lock().unwrap();
    let previous = env::var_os("NOLA_LLM_DNS_FALLBACKS");
    env::set_var("NOLA_LLM_DNS_FALLBACKS", "api.deepseek.com=211.95.70.108");

    let addrs = resolve_dns_fallbacks_for_host("API.DEEPSEEK.COM", 443);
    assert_eq!(addrs.len(), 1);
    assert_eq!(
        addrs[0],
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(211, 95, 70, 108)), 443)
    );

    if let Some(value) = previous {
        env::set_var("NOLA_LLM_DNS_FALLBACKS", value);
    } else {
        env::remove_var("NOLA_LLM_DNS_FALLBACKS");
    }
}

#[test]
fn thinking_capability_false_does_not_disable_provider_default() {
    without_provider_hint(|| {
        let cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "https://api.deepseek.com".to_string(),
            api_key: "key".to_string(),
            model: "deepseek-v4-pro".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 16_384,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
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
            parallel_tool_calls: None,
        };
        let mut body = json!({"messages": [], "stream": true});

        apply_thinking_policy(&cfg, &mut body);

        assert!(body.get("thinking").is_none());
        assert!(body.get("enable_thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    });
}

#[test]
fn main_turn_prompt_cache_identity_uses_core_digest_and_namespace() {
    without_provider_hint(|| {
        let mut cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: "key".to_string(),
            model: "gpt-5".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 16_384,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            openai_prompt_cache: true,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: Some("project-a".to_string()),
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };
        let digest = "0123456789abcdef".repeat(4);
        apply_main_turn_prompt_cache_identity(&mut cfg, &digest);
        let first = cfg.openai_prompt_cache_key.clone().expect("derived key");
        assert_eq!(first.len(), 64);
        assert_ne!(first, digest);

        cfg.openai_prompt_cache_key = None;
        cfg.model = "gpt-5-mini".to_string();
        apply_main_turn_prompt_cache_identity(&mut cfg, &digest);
        assert_ne!(cfg.openai_prompt_cache_key.as_deref(), Some(first.as_str()));
    });
}

#[test]
fn explicit_prompt_cache_key_overrides_core_identity() {
    let mut cfg = LlmConfig {
        siwc: None, native_model_route: None,
        compaction_output_budget: None,
        chat_output_token_field: Default::default(),
        provider: None,
        fallbacks: Vec::new(),
        endpoint: "https://api.openai.com/v1".to_string(),
        api_key: "key".to_string(),
        model: "gpt-5".to_string(),
        temperature: 0.7,
        top_p: 0.9,
        max_output_tokens: 16_384,
        max_retries: crate::llm::default_llm_max_retries(),
        thinking_enabled: false, reasoning_summary_supported: None,
        reasoning_selection: None,
        sampling_override: false,
        generation_options: Default::default(),
        cancellation: Default::default(),
        openai_prompt_cache: true,
        openai_prompt_cache_key: Some("configured-override".to_string()),
        openai_prompt_cache_key_prefix: Some("ignored".to_string()),
        openai_prompt_cache_retention: None,
        openai_prompt_cache_history_breakpoint: false,
        openai_prompt_cache_history_breakpoint_endpoints: None,
        parallel_tool_calls: None,
    };
    apply_main_turn_prompt_cache_identity(&mut cfg, &"f".repeat(64));
    assert_eq!(
        cfg.openai_prompt_cache_key.as_deref(),
        Some("configured-override")
    );
}

#[test]
fn deepseek_automatic_reasoning_preserves_provider_default() {
    without_provider_hint(|| {
        let cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "https://api.deepseek.com".to_string(),
            api_key: "key".to_string(),
            model: "deepseek-v4-pro".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 16_384,
            max_retries: crate::llm::default_llm_max_retries(),
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
            parallel_tool_calls: None,
        };
        let mut body = json!({"messages": [], "stream": true});

        apply_thinking_policy(&cfg, &mut body);

        assert!(body.get("thinking").is_none());
        assert!(body.get("enable_thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    });
}

#[test]
fn deepseek_explicit_none_is_the_only_thinking_disable() {
    without_provider_hint(|| {
        let mut cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "https://api.deepseek.com/chat/completions".into(),
            api_key: String::new(),
            model: "deepseek-flash".into(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 4096,
            max_retries: 0,
            thinking_enabled: false, reasoning_summary_supported: None,
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
            parallel_tool_calls: None,
        };
        let mut automatic = json!({"messages": []});
        apply_thinking_policy(&cfg, &mut automatic);
        assert!(automatic.get("thinking").is_none());
        cfg.reasoning_selection = Some(nola_core::agent::reasoning::ReasoningSelection::Effort {
            value: "none".into(),
        });
        let mut disabled = json!({"messages": []});
        apply_thinking_policy(&cfg, &mut disabled);
        assert_eq!(disabled["thinking"]["type"], "disabled");
    });
}

#[test]
fn thinking_policy_uses_enable_thinking_for_qwen_style_providers() {
    without_provider_hint(|| {
        let cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1".to_string(),
            api_key: "key".to_string(),
            model: "qwen3-coder".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 16_384,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
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
            parallel_tool_calls: None,
        };
        let mut body = json!({"messages": [], "stream": true});

        apply_thinking_policy(&cfg, &mut body);

        assert_eq!(body["enable_thinking"], false);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    });
}

#[test]
fn thinking_policy_omits_vendor_fields_for_openai() {
    without_provider_hint(|| {
        let cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: "key".to_string(),
            model: "gpt-4.1".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 16_384,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: true, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            openai_prompt_cache: true,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        };
        let mut body = json!({"messages": [], "stream": true});

        apply_thinking_policy(&cfg, &mut body);

        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("enable_thinking").is_none());
        assert!(body.get("chat_template_kwargs").is_none());
    });
}

#[test]
fn anthropic_cache_marker_on_latest_message_by_default() {
    without_provider_hint(|| {
        let mut messages = vec![
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "first question"}]
            }),
            json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "analysis"}]
            }),
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "latest question"}]
            }),
        ];

        attach_anthropic_cache_breakpoint(&mut messages, false, None);

        assert_eq!(
            messages[2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert!(messages[1]["content"][0].get("cache_control").is_none());
    });
}

#[test]
fn anthropic_cache_marker_uses_second_last_when_skip_cache_write() {
    without_provider_hint(|| {
        let mut messages = vec![
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "first question"}]
            }),
            json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "analysis"}]
            }),
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "latest question"}]
            }),
        ];

        attach_anthropic_cache_breakpoint(&mut messages, true, None);

        assert_eq!(
            messages[1]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert!(messages[2]["content"][0].get("cache_control").is_none());
    });
}

#[test]
fn anthropic_cache_marker_uses_configured_ttl() {
    without_provider_hint(|| {
        let mut messages = vec![json!({
            "role": "user",
            "content": [{"type": "text", "text": "ping"}]
        })];

        attach_anthropic_cache_breakpoint(&mut messages, false, Some("1h"));

        assert_eq!(messages[0]["content"][0]["cache_control"]["ttl"], "1h");
    });
}

#[test]
fn anthropic_cache_fork_without_a_shared_message_does_not_cache_its_tail() {
    let mut messages = vec![json!({"role":"user", "content":[{"type":"text","text":"one-off summary"}]})];
    let original = messages.clone();
    attach_anthropic_cache_breakpoint(&mut messages, true, None);
    assert_eq!(messages, original);
}

#[test]
fn anthropic_cache_covers_the_complete_multimodal_message() {
    let mut messages = vec![json!({"role":"user", "content":[
        {"type":"text", "text":"Inspect this"},
        {"type":"image", "source":{"type":"base64","media_type":"image/png","data":"fixture"}}
    ]})];
    attach_anthropic_cache_breakpoint(&mut messages, false, None);
    assert_eq!(messages[0]["content"].as_array().unwrap().len(), 2);
    assert!(messages[0]["content"][0].get("cache_control").is_none());
    assert_eq!(messages[0]["content"][1]["cache_control"]["type"], "ephemeral");
}

#[test]
fn anthropic_cache_marker_does_not_rewrite_prior_tool_results() {
    without_provider_hint(|| {
        let mut messages = vec![
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "start"}]
            }),
            json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "first answer"}]
            }),
            json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tool_123",
                    "content": "result"
                }]
            }),
            json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "latest"}]
            }),
        ];

        attach_anthropic_cache_breakpoint(&mut messages, false, None);

        assert!(messages[2]["content"][0].get("cache_reference").is_none());
        assert_eq!(messages[2]["content"][0], json!({
            "type":"tool_result", "tool_use_id":"tool_123", "content":"result"
        }));
    });
}

#[test]
fn llama_cpp_required_tools_stream_by_default() {
    without_provider_hint(|| {
        let cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "http://192.168.31.18:8080/v1".to_string(),
            api_key: "key".to_string(),
            model: "qwen3.6-35b-a3b".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 16_384,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
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
            parallel_tool_calls: None,
        };

        assert!(!non_stream_required_tools_enabled(&cfg));
    });
}

#[test]
fn request_scoped_provider_selects_anthropic_without_process_state() {
    assert_eq!(
        detect_chat_provider(Some("anthropic"), "http://127.0.0.1:8000/v1", "custom"),
        ChatProvider::Anthropic
    );
}

#[test]
fn openai_compatible_transport_does_not_hide_dashscope_flavor() {
    assert_eq!(
        detect_chat_provider(
            Some("openai"),
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "qwen3.5-35b-a3b",
        ),
        ChatProvider::OpenAiCompatible(OpenAiFlavor::DashScope)
    );
}

#[test]
fn detects_qwen_local_as_llamacpp_compatible() {
    without_provider_hint(|| {
        assert_eq!(
            detect_chat_provider(None, "http://192.168.31.18:8080/v1", "qwen3.6-35b-a3b"),
            ChatProvider::OpenAiCompatible(OpenAiFlavor::LlamaCpp)
        );
    });
}

#[test]
fn anthropic_automatic_reasoning_leaves_provider_default_untouched() {
    let mut cfg = LlmConfig {
        siwc: None, native_model_route: None,
        compaction_output_budget: None,
        chat_output_token_field: Default::default(),
        provider: Some("anthropic".into()),
        fallbacks: Vec::new(),
        endpoint: "https://fixture.invalid/v1".into(),
        api_key: String::new(),
        model: "claude-fixture".into(),
        temperature: 0.7,
        top_p: 0.9,
        max_output_tokens: 8192,
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
    };
    let automatic =
        anthropic_request_body(&cfg, &[json!({"role":"user","content":"Hi"})], &[], "auto")
            .unwrap();
    assert!(automatic.get("thinking").is_none());
    cfg.reasoning_selection =
        Some(nola_core::agent::reasoning::ReasoningSelection::Budget { tokens: 2048 });
    let selected =
        anthropic_request_body(&cfg, &[json!({"role":"user","content":"Hi"})], &[], "auto")
            .unwrap();
    assert_eq!(selected["thinking"]["budget_tokens"], 2048);
}

#[test]
fn anthropic_body_converts_system_tools_and_tool_results() {
    without_provider_hint(|| {
        let cfg = LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint: "https://api.anthropic.com".to_string(),
            api_key: "key".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 16_384,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
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
            parallel_tool_calls: None,
        };
        let messages = vec![
            json!({"role": "system", "content": "Use tools."}),
            json!({"role": "user", "content": "Read Cargo.toml"}),
            json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {"name": "read_file", "arguments": "{\"path\":\"Cargo.toml\"}"}
                }]
            }),
            json!({"role": "tool", "tool_call_id": "call_1", "content": "[package]\nname=\"nola_cli\""}),
        ];
        let tools = vec![json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file",
                "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}
            }
        })];

        let body = anthropic_request_body(&cfg, &messages, &tools, "required").unwrap();

        assert_eq!(
            body["system"],
            json!([{"type":"text", "text":"Use tools."}])
        );
        assert_eq!(body["tool_choice"]["type"], "any");
        let mut serial = cfg.clone();
        serial.parallel_tool_calls = Some(false);
        let serial_body = anthropic_request_body(&serial, &messages, &tools, "auto").unwrap();
        assert_eq!(serial_body["tool_choice"]["type"], "auto");
        assert_eq!(
            serial_body["tool_choice"]["disable_parallel_tool_use"],
            true
        );
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(
            body["messages"][1]["content"][0]["input"]["path"],
            "Cargo.toml"
        );
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "call_1");
    });
}

#[test]
fn anthropic_stream_parser_collects_text_reasoning_and_tools() {
    let mut choice = StreamedChoice::default();
    let mut slots = BTreeMap::<usize, ToolSlot>::new();
    let mut events = Vec::<String>::new();

    parse_anthropic_stream_event(
        &json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {"type": "thinking", "thinking": "plan"}
        }),
        &mut choice,
        &mut slots,
        &mut |event| {
            if let LlmEvent::ReasoningDelta(text) = event {
                events.push(format!("r:{text}"));
            }
            Ok(())
        },
    )
    .unwrap();
    parse_anthropic_stream_event(
        &json!({
            "type": "content_block_delta",
            "index": 1,
            "delta": {"type": "text_delta", "text": "pong"}
        }),
        &mut choice,
        &mut slots,
        &mut |event| {
            if let LlmEvent::TextDelta(text) = event {
                events.push(format!("t:{text}"));
            }
            Ok(())
        },
    )
    .unwrap();
    parse_anthropic_stream_event(
        &json!({
            "type": "content_block_start",
            "index": 2,
            "content_block": {"type": "tool_use", "id": "toolu_1", "name": "read_file", "input": {}}
        }),
        &mut choice,
        &mut slots,
        &mut |_| Ok(()),
    )
    .unwrap();
    parse_anthropic_stream_event(
        &json!({
            "type": "content_block_delta",
            "index": 2,
            "delta": {"type": "input_json_delta", "partial_json": "{\"path\":\"Cargo.toml\"}"}
        }),
        &mut choice,
        &mut slots,
        &mut |_| Ok(()),
    )
    .unwrap();
    parse_anthropic_stream_event(
        &json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}}),
        &mut choice,
        &mut slots,
        &mut |_| Ok(()),
    )
    .unwrap();

    assert_eq!(choice.reasoning, "plan");
    assert_eq!(choice.content, "pong");
    assert_eq!(choice.finish_reason, "tool_calls");
    assert_eq!(slots[&2].id, "toolu_1");
    assert_eq!(slots[&2].name, "read_file");
    assert_eq!(slots[&2].arguments, "{\"path\":\"Cargo.toml\"}");
    assert_eq!(events, vec!["r:plan", "t:pong"]);
}

#[test]
fn anthropic_stream_parser_preserves_typed_error_envelope() {
    let mut choice = StreamedChoice::default();
    let mut slots = BTreeMap::<usize, ToolSlot>::new();
    let error = parse_anthropic_stream_event(
        &json!({
            "type": "error",
            "error": {
                "type": "prompt_too_long",
                "message": "入力が長すぎます"
            }
        }),
        &mut choice,
        &mut slots,
        &mut |_| Ok(()),
    )
    .expect_err("Anthropic SSE errors must terminate the provider attempt");

    assert_eq!(
        super::structured_provider_error_is_context_limit(&error),
        Some(true)
    );
    assert!(choice.content.is_empty());
    assert!(slots.is_empty());
}

#[test]
fn model_debug_payloads_keep_metrics_without_message_content_or_secrets() {
    let cfg = LlmConfig {
        siwc: None, native_model_route: None,
        compaction_output_budget: None,
        chat_output_token_field: Default::default(),
        provider: None,
        fallbacks: Vec::new(),
        endpoint: "https://user:credential-secret@example.invalid/private-endpoint?key=query-secret#fragment-secret".to_string(),
        api_key: "api-key-secret".to_string(),
        model: "/private/model-secret".to_string(),
        temperature: 0.7,
        top_p: 0.9,
        max_output_tokens: 16_384,
        max_retries: crate::llm::default_llm_max_retries(),
        thinking_enabled: false, reasoning_summary_supported: None,
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
        parallel_tool_calls: None,
    };
    let choice = StreamedChoice {
        provider_raw_usage: None,
        request_snapshot: None,
        response_items: Vec::new(),
        content: "checking the source".to_string(),
        reasoning: "need a file read".to_string(),
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: "{\"path\":\"src/main.rs\"}".to_string(),
        }],
        finish_reason: "tool_calls".to_string(),
        response_id: Some("chatcmpl-debug".to_string()),
        response_model: Some("resolved-debug-model".to_string()),
        provider_namespace: None,
        provider_model: None,
        usage: LlmUsage::default(),
    };

    let payload = model_response_debug_payload(&cfg, &choice);

    assert_eq!(payload["return_kind"], "tool_calls");
    assert_eq!(payload["content_chars"], choice.content.chars().count());
    assert_eq!(payload["reasoning_json_bytes"], choice.reasoning.len());
    assert_eq!(payload["finish_reason"], "tool_calls");
    assert_eq!(payload["endpoint_host"], "example.invalid");
    assert_eq!(payload["tool_call_count"], 1);
    assert_eq!(
        payload["tool_calls"][0]["arguments_json_bytes"],
        choice.tool_calls[0].arguments.len()
    );
    let messages = vec![json!({"role": "user", "content": [
        {"type": "text", "text": "用户隐私-secret"},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,image-secret"}}
    ]})];
    let tools = vec![json!({"function": {"name": "tool-secret", "description": "schema-secret"}})];
    let request = model_request_debug_payload(
        &cfg,
        &cfg.endpoint,
        &messages,
        &tools,
        "choice-secret",
        "error body-secret",
    );
    assert_eq!(request["tool_choice"], "other");
    assert_eq!(request["message_count"], 1);
    assert_eq!(request["tool_count"], 1);
    assert_eq!(
        request["message_json_bytes"],
        serde_json::to_vec(&messages).unwrap().len()
    );
    assert_eq!(request["detail_bytes"], "error body-secret".len());
    let serialized = format!("{payload}{request}");
    for secret in [
        "secret",
        "checking the source",
        "need a file read",
        "src/main.rs",
        "call_1",
        "read_file",
        "chatcmpl-debug",
        "resolved-debug-model",
    ] {
        assert!(
            !serialized.contains(secret),
            "diagnostic copied private field: {secret}"
        );
    }
    for field in ["content_preview", "reasoning_preview", "model", "endpoint"] {
        assert!(payload.get(field).is_none());
    }
    assert!(payload["tool_calls"][0].get("arguments_preview").is_none());
    let mut unknown_finish = choice;
    unknown_finish.finish_reason = "provider body-secret".into();
    unknown_finish.content = "私密".into();
    let projected = model_response_debug_payload(&cfg, &unknown_finish);
    assert_eq!(projected["finish_reason"], "other");
    assert_eq!(projected["content_chars"], 2);
    assert_eq!(projected["content_json_bytes"], 6);
}

#[test]
fn repetition_guard_allows_structured_long_markdown() {
    let answer = (0..48)
        .map(|index| {
            format!(
                "### Finding {index}\n- status: checked\n- evidence: source line {index}\n- recommendation: keep the parameterized query.\n"
            )
        })
        .collect::<String>();

    assert!(answer.chars().count() > 1_800);
    assert!(!should_stop_repetitive_output(&answer));
}

#[test]
fn repetition_guard_stops_consecutive_long_block_loop() {
    let block = format!(
        "The model is repeating the same paragraph without adding evidence. {}\n",
        "repeat ".repeat(80)
    );
    let answer = format!("{}{}{}", block, block, block);

    assert!(answer.chars().count() > 1_800);
    assert!(should_stop_repetitive_output(&answer));
}
