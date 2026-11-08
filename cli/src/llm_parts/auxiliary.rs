// SPDX-License-Identifier: Apache-2.0
//! Exactly one HTTP sampling request. No retry, fallback, tools, or Agent Run.
use super::contracts::{resolved_model_protocol, LlmConfig};
use nola_domain::generation::ModelProtocol;
use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;

pub(crate) enum AuxiliaryPhysicalOutcome {
    Complete {
        content: Result<String, String>,
        usage: Value,
        finish_reason: Option<String>,
        has_tool_calls: Option<bool>,
    },
    /// No provider completion witness: retain the Running Attempt, forbid resend.
    Unknown,
}

pub(crate) fn auxiliary_usage(raw: &Value, protocol: ModelProtocol) -> Value {
    if raw.to_string().len() > 64 * 1024 {
        return json!({"coverage":"unknown","provider_raw":null,"normalized":null});
    }
    let input = if protocol == ModelProtocol::ChatCompletions {
        raw["prompt_tokens"].as_u64()
    } else {
        raw["input_tokens"].as_u64()
    };
    let output = if protocol == ModelProtocol::ChatCompletions {
        raw["completion_tokens"].as_u64()
    } else {
        raw["output_tokens"].as_u64()
    };
    let cached = match protocol {
        ModelProtocol::ChatCompletions => raw["prompt_tokens_details"]["cached_tokens"].as_u64(),
        ModelProtocol::Responses | ModelProtocol::SiwcResponses => raw["input_tokens_details"]["cached_tokens"].as_u64(),
        ModelProtocol::Anthropic => raw["cache_read_input_tokens"].as_u64(),
    };
    let write = if protocol == ModelProtocol::Anthropic {
        raw["cache_creation_input_tokens"].as_u64()
    } else {
        raw["input_tokens_details"]["cache_write_tokens"].as_u64()
    };
    let input = if protocol == ModelProtocol::Anthropic {
        input
            .zip(cached)
            .zip(write)
            .and_then(|((base, read), write)| base.checked_add(read)?.checked_add(write))
    } else {
        input
    };
    if let Some((input, output)) = input.zip(output).filter(|(input, _)| {
        cached.is_none_or(|read| read <= *input) && write.is_none_or(|write| write <= *input)
    }) {
        json!({"coverage":"known","provider_raw":raw,"normalized":{"input_tokens":input,"output_tokens":output,
            "cached_input_tokens":cached,"cache_write_input_tokens":write}})
    } else {
        json!({"coverage":"unknown","provider_raw":raw,"normalized":null})
    }
}

/// Provider evidence only; absent termination metadata remains unknown.
fn auxiliary_completion(response: &Value, protocol: ModelProtocol) -> (Option<String>, Option<bool>) {
    match protocol {
        ModelProtocol::ChatCompletions => {
            let choice = &response["choices"][0];
            (choice["finish_reason"].as_str().map(str::to_owned),
             choice["message"].as_object().map(|message| {
                message.get("tool_calls").and_then(Value::as_array).is_some_and(|calls| !calls.is_empty())
                    || message.get("function_call").is_some_and(|call| !call.is_null())
             }))
        }
        ModelProtocol::Responses | ModelProtocol::SiwcResponses => (
            response["status"].as_str().map(str::to_owned),
            response["output"].as_array().map(|items| items.iter().any(|item| {
                item["type"].as_str().is_some_and(|kind| kind.ends_with("_call"))
            }))),
        ModelProtocol::Anthropic => (
            response["stop_reason"].as_str().map(str::to_owned),
            response["content"].as_array().map(|items| items.iter().any(|item| item["type"] == "tool_use"))),
    }
}

pub(crate) fn invoke_auxiliary_once(
    cfg: &LlmConfig,
    messages: &[Value],
    duration_ms: u64,
) -> Result<AuxiliaryPhysicalOutcome, String> {
    if cfg.max_retries != 0 || !cfg.fallbacks.is_empty() || cfg.native_model_route.is_some() {
        return Err("auxiliary transport requires one explicit HTTP target".into());
    }
    let protocol = resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint);
    if protocol == ModelProtocol::SiwcResponses {
        let body = super::responses::siwc_responses_body(cfg, messages, &[], "none")?;
        return match super::responses::stream_responses_body_with_timeout(cfg, &body,
            Some(Duration::from_millis(duration_ms.clamp(1, 3_600_000))), |_| Ok(())) {
            Ok(choice) => Ok(AuxiliaryPhysicalOutcome::Complete {
                finish_reason: Some(choice.finish_reason.clone()),
                has_tool_calls: Some(!choice.tool_calls.is_empty()),
                usage: auxiliary_usage(choice.provider_raw_usage.as_ref().unwrap_or(&Value::Null), protocol),
                content: if choice.content.trim().is_empty() { Err("auxiliary provider returned no complete text result".into()) } else { Ok(choice.content) },
            }),
            Err(super::contracts::ProviderAttemptError::Http { .. }) => Ok(AuxiliaryPhysicalOutcome::Complete {
                finish_reason: None, has_tool_calls: None,
                content: Err("auxiliary provider rejected request".into()),
                usage: auxiliary_usage(&Value::Null, protocol),
            }),
            Err(_) => Ok(AuxiliaryPhysicalOutcome::Unknown),
        };
    }
    let mut body = match protocol {
        ModelProtocol::ChatCompletions => {
            super::streaming::openai_request_body(cfg, messages, &[], "none")?
        }
        ModelProtocol::Responses => super::responses::responses_body(cfg, messages, &[], "none")?,
        ModelProtocol::SiwcResponses => return Err("SIWC requires streaming auxiliary transport".into()),
        ModelProtocol::Anthropic => {
            super::provider_protocols::anthropic_request_body(cfg, messages, &[], "none")?
        }
    };
    body["stream"] = json!(false);
    if let Some(object) = body.as_object_mut() {
        object.remove("stream_options");
    }
    let url = super::transport::normalized_model_endpoint(&cfg.endpoint, protocol)?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_millis(duration_ms.clamp(1, 3_600_000)))
        .build()
        .map_err(|_| "auxiliary HTTP client unavailable")?;
    if cfg.cancellation.is_requested() {
        return Err("auxiliary cancelled before dispatch".into());
    }
    let mut request = client
        .post(url)
        .header("Accept", "application/json")
        .json(&body);
    if protocol == ModelProtocol::Anthropic {
        request = request
            .header("anthropic-version", "2023-06-01")
            .header("x-api-key", &cfg.api_key);
    } else if !cfg.api_key.is_empty() {
        request = request.bearer_auth(&cfg.api_key);
    }
    let response = match request.send() {
        Ok(response) => response,
        Err(_) => return Ok(AuxiliaryPhysicalOutcome::Unknown),
    };
    let success = response.status().is_success();
    let mut bytes = Vec::new();
    if response
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Ok(AuxiliaryPhysicalOutcome::Unknown);
    }
    if bytes.len() > 4 * 1024 * 1024 {
        return Ok(AuxiliaryPhysicalOutcome::Complete {
            finish_reason: None, has_tool_calls: None,
            content: Err("auxiliary response exceeds limit".into()),
            usage: json!({"coverage":"unknown","provider_raw":null,"normalized":null}),
        });
    }
    let response: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let usage = auxiliary_usage(&response["usage"], protocol);
    let (finish_reason, has_tool_calls) = auxiliary_completion(&response, protocol);
    let content = if !success {
        Err("auxiliary provider rejected request".into())
    } else {
        match protocol {
            ModelProtocol::ChatCompletions => response["choices"][0]["message"]["content"]
                .as_str()
                .map(str::to_owned)
                .filter(|_| response["choices"][0]["finish_reason"] == "stop"),
            ModelProtocol::SiwcResponses => return Err("SIWC requires streaming auxiliary transport".into()),
            ModelProtocol::Responses => response["output"]
                .as_array()
                .filter(|_| response["status"] == "completed")
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item["content"].as_array())
                        .flatten()
                        .filter(|item| item["type"] == "output_text")
                        .filter_map(|item| item["text"].as_str())
                        .collect::<String>()
                }),
            ModelProtocol::Anthropic => response["content"]
                .as_array()
                .filter(|_| {
                    response["stop_reason"] == "end_turn"
                        || response["stop_reason"] == "stop_sequence"
                })
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| item["type"] == "text")
                        .filter_map(|item| item["text"].as_str())
                        .collect::<String>()
                }),
        }
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "auxiliary provider returned no complete text result".into())
    };
    Ok(AuxiliaryPhysicalOutcome::Complete { content, usage, finish_reason, has_tool_calls })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_metadata_preserves_provider_evidence_and_unknown() {
        assert_eq!(auxiliary_completion(&json!({"choices":[{"finish_reason":"length","message":{"content":"partial"}}]}), ModelProtocol::ChatCompletions),
            (Some("length".into()), Some(false)));
        assert_eq!(auxiliary_completion(&json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"call"}]}}]}), ModelProtocol::ChatCompletions),
            (Some("tool_calls".into()), Some(true)));
        assert_eq!(auxiliary_completion(&json!({}), ModelProtocol::ChatCompletions), (None, None));
        assert_eq!(auxiliary_completion(&json!({"status":"incomplete","output":[]}), ModelProtocol::Responses),
            (Some("incomplete".into()), Some(false)));
        assert_eq!(auxiliary_completion(&json!({"stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}), ModelProtocol::Anthropic),
            (Some("end_turn".into()), Some(false)));
    }
    #[test]
    fn absent_usage_is_unknown_and_measured_cache_is_not_fabricated() {
        assert_eq!(
            auxiliary_usage(&Value::Null, ModelProtocol::ChatCompletions)["coverage"],
            "unknown"
        );
        let usage = auxiliary_usage(
            &json!({"prompt_tokens":100,"completion_tokens":9}),
            ModelProtocol::ChatCompletions,
        );
        assert_eq!(usage["normalized"]["input_tokens"], 100);
        assert!(usage["normalized"]["cached_input_tokens"].is_null());
        let usage = auxiliary_usage(
            &json!({"input_tokens":100,"output_tokens":9,
            "cache_read_input_tokens":30,"cache_creation_input_tokens":20}),
            ModelProtocol::Anthropic,
        );
        assert_eq!(usage["normalized"]["input_tokens"], 150);
        assert_eq!(usage["normalized"]["cached_input_tokens"], 30);
    }

    #[test]
    fn complete_http_failure_keeps_real_usage_and_does_not_retry() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", server.server_addr());
        let worker = std::thread::spawn(move || {
            let mut request = server.recv().unwrap();
            let mut bytes = String::new();
            request.as_reader().read_to_string(&mut bytes).unwrap();
            let request_json: Value = serde_json::from_str(&bytes).unwrap();
            assert_eq!(request_json["stream"], false);
            assert!(request_json.get("tools").is_none());
            request
                .respond(
                    tiny_http::Response::from_string(
                        json!({"error":{"message":"rejected"},
                "usage":{"prompt_tokens":27,"completion_tokens":3}})
                        .to_string(),
                    )
                    .with_status_code(429),
                )
                .unwrap();
            assert!(server.try_recv().unwrap().is_none());
        });
        let config = LlmConfig {
            siwc: None, native_model_route: None,
            provider: Some("openai-compatible".into()),
            fallbacks: vec![],
            endpoint,
            api_key: String::new(),
            model: "auxiliary-test".into(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 8192,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            max_retries: 0,
            parallel_tool_calls: Some(false),
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
        };
        let outcome =
            invoke_auxiliary_once(&config, &[json!({"role":"user","content":"summary"})], 1000)
                .unwrap();
        match outcome {
            AuxiliaryPhysicalOutcome::Complete { content, usage, .. } => {
                assert!(content.is_err());
                assert_eq!(usage["coverage"], "known");
                assert_eq!(usage["provider_raw"]["prompt_tokens"], 27);
                assert_eq!(usage["normalized"]["output_tokens"], 3);
            }
            AuxiliaryPhysicalOutcome::Unknown => {
                panic!("a complete HTTP response is a physical boundary")
            }
        }
        worker.join().unwrap();
    }
}
