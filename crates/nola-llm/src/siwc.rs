// SPDX-License-Identifier: Apache-2.0
//! SIWC Responses wire adapter (`siwc_responses`).
//! Uses official `https://api.openai.com/v1/responses`, enforces `stream: true` and `store: false`,
//! strips forbidden sampling/token fields, strips internal `x-nola-read-only` schema extensions,
//! and wraps local function tools inside the top-level `nola` namespace.

use std::collections::BTreeMap;
use std::io::BufRead;

use nola_types::{
    GenerationParams, ModelProtocol, ModelRouteConfig, ModelStreamDelta, NormalizedModelResponse,
    StructuredModelRequest,
};
use serde_json::{json, Value};

use crate::adapter::{BuiltHttpRequest, LlmError, ProtocolMediaLimits, ProviderWireAdapter};
use crate::openai_responses::{build_responses_body, decode_responses_body, parse_responses_sse};

pub struct SiwcResponsesAdapter;

const OFFICIAL_SIWC_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";

fn siwc_endpoint(base: &str) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("https://api.openai.com")
        || trimmed.eq_ignore_ascii_case("https://api.openai.com/v1")
    {
        return OFFICIAL_SIWC_RESPONSES_URL.to_string();
    }
    if trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/responses")
    } else {
        format!("{trimmed}/v1/responses")
    }
}

/// Recursively strip internal `x-nola-read-only` metadata extensions from a JSON value.
fn strip_nola_extensions(val: &mut Value) {
    match val {
        Value::Object(map) => {
            map.remove("x-nola-read-only");
            for v in map.values_mut() {
                strip_nola_extensions(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                strip_nola_extensions(v);
            }
        }
        _ => {}
    }
}

/// Apply SIWC-specific wire constraints on top of the canonical Responses payload:
/// - Enforce `stream: true` and `store: false`.
/// - Strip forbidden fields (`max_output_tokens`, `temperature`, `top_p`, `seed`).
/// - Strip internal `x-nola-read-only` extension fields and wrap function tools in the `nola` namespace.
fn apply_siwc_payload_contract(body: &mut Value) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    obj.insert("stream".into(), Value::Bool(true));
    obj.insert("store".into(), Value::Bool(false));
    obj.remove("max_output_tokens");
    obj.remove("temperature");
    obj.remove("top_p");
    obj.remove("seed");

    if let Some(tools_val) = obj.remove("tools") {
        if let Value::Array(raw_tools) = tools_val {
            let mut fn_tools = Vec::new();
            let mut other_tools = Vec::new();
            for mut tool in raw_tools {
                strip_nola_extensions(&mut tool);
                let is_function = tool
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|t| t == "function")
                    .unwrap_or(false);
                if is_function {
                    if let Some(tobj) = tool.as_object_mut() {
                        tobj.entry("strict").or_insert(Value::Bool(false));
                    }
                    fn_tools.push(tool);
                } else {
                    other_tools.push(tool);
                }
            }
            let mut wrapped = other_tools;
            if !fn_tools.is_empty() {
                wrapped.push(json!({
                    "type": "namespace",
                    "name": "nola",
                    "description": "Nola workspace tools",
                    "tools": fn_tools
                }));
            }
            if !wrapped.is_empty() {
                obj.insert("tools".into(), Value::Array(wrapped));
            }
        }
    }
}

impl ProviderWireAdapter for SiwcResponsesAdapter {
    fn protocol(&self) -> ModelProtocol {
        ModelProtocol::SiwcResponses
    }

    fn media_limits(&self) -> ProtocolMediaLimits {
        ProtocolMediaLimits::OPENAI
    }

    fn build_request(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
    ) -> Result<BuiltHttpRequest, LlmError> {
        self.build_request_with_params(route, req, &GenerationParams::default(), stream)
    }

    fn build_request_with_params(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        params: &GenerationParams,
        _stream: bool,
    ) -> Result<BuiltHttpRequest, LlmError> {
        let mut body = build_responses_body(
            ModelProtocol::SiwcResponses,
            route,
            req,
            params,
            true,
            true,
        )?;
        apply_siwc_payload_contract(&mut body);
        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        if !route.api_key.trim().is_empty() {
            headers.insert(
                "authorization".into(),
                format!("Bearer {}", route.api_key.trim()),
            );
        }
        Ok(BuiltHttpRequest {
            url: siwc_endpoint(&route.endpoint),
            headers,
            body,
        })
    }

    fn decode_response(
        &self,
        route: &ModelRouteConfig,
        body: &Value,
    ) -> Result<NormalizedModelResponse, LlmError> {
        decode_responses_body(ModelProtocol::SiwcResponses, route, body)
    }

    fn parse_sse_stream(
        &self,
        route: &ModelRouteConfig,
        reader: &mut dyn BufRead,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        parse_responses_sse(ModelProtocol::SiwcResponses, route, reader, on_delta)
    }
}
