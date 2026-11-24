// SPDX-License-Identifier: Apache-2.0
//! SIWC Responses wire adapter (`siwc_responses`).
//! Reuses canonical OpenAI Responses encoding/decoding with SIWC endpoint & `store: false`.

use std::collections::BTreeMap;
use std::io::BufRead;

use nola_types::{
    ModelProtocol, ModelRouteConfig, ModelStreamDelta, NormalizedModelResponse,
    StructuredModelRequest,
};
use serde_json::Value;

use crate::adapter::{BuiltHttpRequest, LlmError, ProtocolMediaLimits, ProviderWireAdapter};
use crate::openai_responses::{build_responses_body, decode_responses_body, parse_responses_sse};

pub struct SiwcResponsesAdapter;

fn siwc_endpoint(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/codex/responses") || trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/backend-api/codex/responses")
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
        let body = build_responses_body(ModelProtocol::SiwcResponses, route, req, stream, true)?;
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
