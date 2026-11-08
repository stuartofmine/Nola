// SPDX-License-Identifier: Apache-2.0
//! Pure, shared profile resolution. Credentials and physical Host state stay outside Core.
use nola_domain::{
    generation::{GenerationOptions, ReasoningSelection},
    model_runtime::RuntimeModelCapabilities,
    reasoning_tiers::ReasoningTier,
};
use serde_json::{json, Value};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ResolvedModelRuntime {
    pub provider: Option<String>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u64>,
    pub context_window_tokens: Option<usize>,
    pub chat_output_token_field: crate::ChatOutputTokenField,
    pub reasoning_selection: Option<ReasoningSelection>,
    pub reasoning_tier: ReasoningTier,
    pub sampling_override: bool,
    pub generation_options: GenerationOptions,
    pub model_capabilities: RuntimeModelCapabilities,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolvedRemoteModelProfile {
    pub selected_profile: nola_host_protocol::HostModelProfileReference,
    pub endpoint: String,
    pub model: String,
    pub runtime: ResolvedModelRuntime,
}
impl ResolvedModelRuntime {
    pub fn with_model_request(&self, request: &serde_json::Value) -> Result<Self, String> {
        let object = request
            .as_object()
            .ok_or("model request must be an object")?;
        for obsolete in ["topP", "maxTokens", "contextWindowTokens"] {
            if object.contains_key(obsolete) {
                return Err(format!("unsupported model request field {obsolete}"));
            }
        }
        let mut result = self.clone();
        if let Some(value) = request.get("reasoning_tier") {
            result.reasoning_tier = serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid reasoning_tier: {error}"))?;
        }
        if let Some(value) = request.get("reasoning_selection") {
            result.reasoning_selection = serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid reasoning_selection: {error}"))?;
        }
        if let Some(value) = request.get("generation_options") {
            result.generation_options = serde_json::from_value(value.clone())
                .map_err(|error| format!("invalid generation_options: {error}"))?;
        }
        result.generation_options.validate()?;
        if let Some(value) = request.get("sampling_override") {
            result.sampling_override =
                value.as_bool().ok_or("sampling_override must be boolean")?;
        } else if ["temperature", "top_p"]
            .iter()
            .any(|key| request.get(key).is_some())
        {
            result.sampling_override = true;
        }
        for (key, range) in [("temperature", 0.0..=2.0), ("top_p", 0.0..=1.0)] {
            if let Some(value) = request.get(key) {
                let number = value
                    .as_f64()
                    .filter(|v| range.contains(v))
                    .ok_or_else(|| format!("invalid {key}"))?;
                if key == "temperature" {
                    result.temperature = Some(number);
                } else {
                    result.top_p = Some(number);
                }
            }
        }
        if let Some(value) = request.get("max_output_tokens") {
            result.max_output_tokens = Some(
                value
                    .as_u64()
                    .filter(|v| (64..=1_000_000).contains(v))
                    .ok_or("invalid max_output_tokens")?,
            );
        }
        if let Some(value) = request.get("max_context_tokens") {
            result.context_window_tokens = Some(
                value
                    .as_u64()
                    .filter(|v| (512..=4_000_000).contains(v))
                    .ok_or("invalid max_context_tokens")? as usize,
            );
        }
        if let Some(protocol) = request.get("model_protocol") {
            use nola_domain::generation::ModelProtocol;
            let protocol =
                ModelProtocol::parse(protocol.as_str().ok_or("unsupported model_protocol")?)?;
            result.provider = Some(
                match protocol {
                    ModelProtocol::Responses => "openai-responses",
                    ModelProtocol::SiwcResponses => "siwc_responses",
                    ModelProtocol::Anthropic => "anthropic",
                    ModelProtocol::ChatCompletions => "openai-compatible",
                }
                .into(),
            );
            result.model_capabilities.reasoning_continuity =
                Some(protocol == ModelProtocol::Responses);
        }
        if let Some(field) = request.get("chat_output_token_field") {
            result.chat_output_token_field = serde_json::from_value(field.clone())
                .map_err(|_| "invalid chat_output_token_field")?;
        }
        if let Some(selection) = &result.reasoning_selection {
            selection.validate(result.max_output_tokens.unwrap_or(16384))?;
            result.model_capabilities.reasoning = Some(true);
        }
        Ok(result)
    }
}
fn reasoning_declaration(
    source: &Value,
) -> Result<nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration, String> {
    use nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration;
    match source["control"].as_str() {
        None if source.is_null() => Ok(ReasoningCapabilityDeclaration::Automatic {}),
        Some("automatic") => Ok(ReasoningCapabilityDeclaration::Automatic {}),
        Some("effort") => Ok(ReasoningCapabilityDeclaration::Effort {
            values: serde_json::from_value(source["efforts"].clone())
                .map_err(|error| error.to_string())?,
        }),
        Some("budget") => Ok(ReasoningCapabilityDeclaration::Budget {
            tokens: serde_json::from_value(source["budgets"].clone())
                .map_err(|error| error.to_string())?,
        }),
        _ => Err("Invalid model reasoning declaration".into()),
    }
}

pub fn validate_model_request_fields(request: &Value) -> Result<(), String> {
    let object = request
        .as_object()
        .ok_or("model request must be an object")?;
    for field in [
        "endpoint",
        "apiKey",
        "topP",
        "maxTokens",
        "contextWindowTokens",
    ] {
        if object.contains_key(field) {
            return Err(format!("unsupported model request field {field}"));
        }
    }
    Ok(())
}

pub fn model_endpoint_identity(endpoint: &str) -> Option<String> {
    let mut url = url::Url::parse(endpoint.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Some(url.to_string())
}

pub fn validate_remote_execution_capabilities(
    protocol: &str,
    reasoning: &Value,
    execution: crate::ModelExecutionCapabilities,
) -> Result<(), String> {
    if execution.sampling_requires_no_reasoning
        && (protocol != "responses"
            || reasoning["control"] != "effort"
            || !reasoning["efforts"]
                .as_array()
                .is_some_and(|values| values.contains(&json!("none"))))
    {
        return Err(
            "Sampling effort restriction requires Responses with declared effort none".into(),
        );
    }
    Ok(())
}

pub fn resolve_remote_profile(
    profile: &crate::ModelProfile,
    request: &Value,
    inherited_reasoning_tier: ReasoningTier,
) -> Result<ResolvedRemoteModelProfile, String> {
    validate_model_request_fields(request)?;
    if profile.runtime != "remote" || !profile.enabled || profile.deleted_at_ms.is_some() {
        return Err("Selected remote model profile is unavailable".into());
    }
    let source: Value =
        serde_json::from_str(&profile.source_json).map_err(|_| "Invalid model source")?;
    if source["endpointEnabled"] == false || source["modelEnabled"] == false {
        return Err("Selected model or endpoint is disabled".into());
    }
    if !source["capabilities"]
        .as_array()
        .is_some_and(|values| values.contains(&json!("chat")))
    {
        return Err("Selected model is not enabled for chat".into());
    }
    let endpoint = source["baseURL"]
        .as_str()
        .ok_or("Model endpoint is missing")?;
    let identity = model_endpoint_identity(endpoint).ok_or("Invalid model endpoint")?;
    for field in ["customEndpoint"] {
        if let Some(value) = request
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            if model_endpoint_identity(value).as_ref() != Some(&identity) {
                return Err("Model profile and request endpoint disagree".into());
            }
        }
    }
    if let Some(model) = request
        .get("model")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        if model != profile.model_name {
            return Err("Model profile and request model disagree".into());
        }
    }
    // Resolve protocol once, before any capability or parameter validation.
    // Session overrides are supported, but cannot be validated against the
    // model default and then sent through a different adapter.
    let protocol = request
        .get("model_protocol")
        .map(|value| value.as_str().ok_or("invalid model protocol override"))
        .transpose()?
        .or_else(|| source["wireProtocol"].as_str())
        .ok_or("Model source wireProtocol is missing")?;
    let resolved_protocol = nola_domain::generation::ModelProtocol::parse(protocol)?;
    let is_siwc = source["wireProtocol"] == "siwc_responses";
    if is_siwc != (source["sourceKind"] == "nola_chatgpt")
        || is_siwc != (protocol == "siwc_responses")
        || (is_siwc
            && (source["sourceKind"] != "nola_chatgpt" || request.get("customApiKey").is_some()))
    {
        return Err(
            "Subscription profiles cannot change billing transport or accept API keys".into(),
        );
    }
    let execution = crate::ModelExecutionCapabilities::from_source(&source)?;
    let parameters: Value =
        serde_json::from_str(&profile.parameters_json).map_err(|_| "Invalid model parameters")?;
    validate_remote_execution_capabilities(protocol, &source["reasoning"], execution)?;
    let mut snapshot = json!({
        "model_protocol":protocol, "reasoning_selection":null,
        "temperature":parameters.get("temperature").cloned().unwrap_or(json!(0.7)),
        "top_p":parameters.get("topP").cloned().unwrap_or(json!(0.9)),
        "max_output_tokens":parameters.get("maxOutputTokens").cloned().unwrap_or(json!(16384)),
        "max_context_tokens":parameters.get("maxContextLength").cloned().unwrap_or(json!(128000)),
        "generation_options":{}
    });
    for (stored, wire) in [
        ("presencePenalty", "presence_penalty"),
        ("frequencyPenalty", "frequency_penalty"),
        ("seed", "seed"),
    ] {
        if let Some(value) = parameters
            .get(stored)
            .filter(|value| !value.is_null() && (stored == "seed" || value.as_f64() != Some(0.0)))
        {
            snapshot["generation_options"][wire] = value.clone();
        }
    }
    // Optional profile values are explicit, including values equal to a
    // local engine's defaults. Protocol validation below owns admissibility.
    for (stored, wire) in [
        ("topK", "top_k"),
        ("repetitionPenalty", "repetition_penalty"),
    ] {
        if let Some(value) = parameters.get(stored).filter(|value| !value.is_null()) {
            snapshot["generation_options"][wire] = value.clone();
        }
    }
    if let Some(stop) = parameters["stopSequences"]
        .as_str()
        .filter(|value| !value.is_empty())
    {
        snapshot["generation_options"]["stop"] = json!(stop
            .lines()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>());
    }
    // The profile owns explicit defaults even when their numbers happen to
    // match Nola's fallback values. Capability, not equality, determines
    // whether the adapter may send them.
    snapshot["sampling_override"] =
        json!(execution.sampling_parameters && !execution.sampling_requires_no_reasoning);
    let base = ResolvedModelRuntime {
        reasoning_tier: inherited_reasoning_tier,
        ..Default::default()
    };
    let mut overrides = request.clone();
    if let Some(options) = request.get("generation_options").and_then(Value::as_object) {
        let mut merged = snapshot["generation_options"].as_object().unwrap().clone();
        merged.extend(
            options
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        overrides["generation_options"] = json!(merged);
    }
    if request.get("sampling_override").is_none()
        && ["temperature", "top_p"]
            .iter()
            .any(|key| request.get(key).is_some())
    {
        overrides["sampling_override"] = json!(true);
    }
    snapshot.as_object_mut().unwrap().extend(
        overrides
            .as_object()
            .ok_or("model request must be an object")?
            .clone(),
    );
    let mut runtime = base.with_model_request(&snapshot)?;
    // The output-limit wire field belongs to the selected model route, not an
    // arbitrary conversation override or the process-wide provider default.
    runtime.chat_output_token_field = execution.chat_output_token_field;
    if runtime.sampling_override && !execution.sampling_parameters {
        return Err("Selected model does not declare sampling parameter support".into());
    }
    if runtime
        .generation_options
        .stop
        .as_ref()
        .is_some_and(|values| values.is_empty())
    {
        runtime.generation_options.stop = None;
    }
    if matches!(protocol, "responses" | "siwc_responses")
        && runtime.generation_options != Default::default()
    {
        return Err(
            "Responses does not support the selected Chat Completions generation options".into(),
        );
    }
    if protocol == "anthropic"
        && (runtime.generation_options.presence_penalty.is_some()
            || runtime.generation_options.frequency_penalty.is_some()
            || runtime.generation_options.repetition_penalty.is_some()
            || runtime.generation_options.seed.is_some())
    {
        return Err("Anthropic does not support the selected generation options".into());
    }
    runtime.model_capabilities.native_tool_calls = Some(execution.tool_calls);
    runtime.model_capabilities.parallel_tool_calls = Some(false);
    runtime.model_capabilities.stream_tool_calls = Some(execution.tool_calls);
    runtime.model_capabilities.vision_input = Some(execution.image_input);
    runtime.model_capabilities.reasoning = Some(execution.reasoning);
    runtime.model_capabilities.reasoning_summary_supported = serde_json::from_value(
        source.get("reasoningSummarySupported").cloned().unwrap_or(Value::Null)
    ).map_err(|_| "Invalid reasoning summary capability")?;
    // All selected remote adapters decode explicit provider reasoning/summary
    // events. Receiving those events does not enable reasoning request options.
    runtime.model_capabilities.stream_reasoning = Some(matches!(
        protocol,
        "chat_completions" | "responses" | "siwc_responses" | "anthropic"
    ));
    runtime.model_capabilities.reasoning_continuity =
        Some(protocol == "responses" && execution.reasoning);
    let output_limit = runtime.max_output_tokens.unwrap_or(16384);
    let stored_selection = if request.get("reasoning_selection").is_none()
        && request.get("reasoning_tier").is_none()
    {
        match parameters.get("reasoningPreference").filter(|value| !value.is_null()) {
            Some(value) if value == "automatic" => Some(Value::Null),
            Some(value) if value.as_str().is_some_and(|value|
                nola_domain::generation::REASONING_EFFORT_VALUES.contains(&value)) =>
                Some(json!({"kind":"effort","value":value})),
            Some(_) => return Err("invalid model reasoning preference".into()),
            None => None,
        }
    } else { None };
    let requested_selection = request.get("reasoning_selection").or(stored_selection.as_ref());
    let requested_tier = request.get("reasoning_tier").filter(|value| !value.is_null())
        .or_else(|| if requested_selection.is_some() { None }
            else { parameters.get("reasoningTier").filter(|value| !value.is_null()) });
    runtime.reasoning_selection = if let Some(tier) = requested_tier {
        if request.get("reasoning_selection").is_some_and(|value| !value.is_null()) {
            return Err("specify a reasoning tier or selection, not both".into());
        }
        runtime.reasoning_tier = serde_json::from_value(tier.clone())
            .map_err(|error| format!("invalid reasoning tier: {error}"))?;
        crate::resolve_model_reasoning_tier(&source["reasoning"], resolved_protocol,
            output_limit, execution.reasoning, runtime.reasoning_tier)?
    } else {
        crate::resolve_model_reasoning(&source["reasoning"], resolved_protocol,
            output_limit, execution.reasoning, requested_selection)?
    };
    // Restore the complete capability contract as well as this turn's choice.
    // A subsequent explicit profile initialization must not fabricate or lose it.
    runtime.model_capabilities.reasoning_declaration =
        Some(reasoning_declaration(&source["reasoning"])?);
    if runtime.sampling_override
        && execution.sampling_requires_no_reasoning
        && !matches!(&runtime.reasoning_selection,
            Some(nola_domain::generation::ReasoningSelection::Effort { value }) if value == "none")
    {
        return Err(
            "Selected model accepts sampling parameters only with explicit reasoning effort none"
                .into(),
        );
    }
    Ok(ResolvedRemoteModelProfile {
        selected_profile: nola_host_protocol::HostModelProfileReference {
            profile_id: profile.id.clone(),
            admitted_revision: profile.revision,
        },
        endpoint: endpoint.to_string(),
        model: profile.model_name.clone(),
        runtime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_override_rejects_old_aliases_without_mutating_the_snapshot() {
        let settings = ResolvedModelRuntime {
            temperature: Some(0.4),
            top_p: Some(0.8),
            ..Default::default()
        };
        for obsolete in [
            json!({"topP":0.5}),
            json!({"maxTokens":4096}),
            json!({"contextWindowTokens":64000}),
        ] {
            assert!(settings.with_model_request(&obsolete).is_err());
        }
        assert!(settings
            .with_model_request(&json!({"temperature":null}))
            .is_err());
        assert_eq!(settings.temperature, Some(0.4));
        let resolved = settings.with_model_request(&json!({"top_p":0.0})).unwrap();
        assert_eq!(resolved.temperature, Some(0.4));
        assert_eq!(resolved.top_p, Some(0.0));
        assert!(resolved.sampling_override);
    }

    fn profile(protocol: &str) -> crate::ModelProfile {
        crate::ModelProfile {
            id: "profile".into(), display_name: "Shared model".into(), provider: "openai-compatible".into(),
            model_name: "model".into(), runtime: "remote".into(),
            source_json: json!({"baseURL":"https://example.test/v1", "wireProtocol":protocol,
                "sourceKind":if protocol == "siwc_responses" { "nola_chatgpt" } else { "api" },
                "capabilities":["chat"], "reasoning":null,
                "executionCapabilities":{"toolCalls":true,"imageInput":false,"samplingParameters":true,
                    "samplingRequiresNoReasoning":false,"reasoning":false}}).to_string(),
            parameters_json: json!({"temperature":0.7,"topP":0.9,"maxOutputTokens":8192,"maxContextLength":64000}).to_string(),
            selected:true, enabled:true, revision:7, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        }
    }

    #[test]
    fn exact_profile_and_conversation_preferences_preserve_all_five_efforts() {
        let mut selected = profile("responses");
        let mut source: Value = serde_json::from_str(&selected.source_json).unwrap();
        source["executionCapabilities"]["reasoning"] = json!(true);
        source["reasoning"] = json!({"control":"effort","efforts":["none","low","medium","high","xhigh"]});
        selected.source_json = source.to_string();
        let mut parameters: Value = serde_json::from_str(&selected.parameters_json).unwrap();
        parameters["reasoningTier"] = json!("detailed");
        for effort in ["none", "low", "medium", "high", "xhigh"] {
            parameters["reasoningPreference"] = json!(effort);
            selected.parameters_json = parameters.to_string();
            let result = resolve_remote_profile(&selected, &json!({}), Default::default()).unwrap();
            assert_eq!(result.runtime.reasoning_selection, Some(ReasoningSelection::Effort { value: effort.into() }));
            let request = session_override_request(&json!({"reasoningPreference":effort}), &source["reasoning"], "responses").unwrap();
            let result = resolve_remote_profile(&selected, &request, Default::default()).unwrap();
            assert_eq!(result.runtime.reasoning_selection, Some(ReasoningSelection::Effort { value: effort.into() }));
        }
        parameters["reasoningPreference"] = json!("automatic");
        selected.parameters_json = parameters.to_string();
        assert_eq!(resolve_remote_profile(&selected, &json!({}), Default::default()).unwrap().runtime.reasoning_selection, None);
        let explicit = json!({"reasoning_selection":{"kind":"effort","value":"low"}});
        assert_eq!(resolve_remote_profile(&selected, &explicit, Default::default()).unwrap().runtime.reasoning_selection,
            Some(ReasoningSelection::Effort { value:"low".into() }));
        parameters["reasoningPreference"] = json!("max");
        selected.parameters_json = parameters.to_string();
        assert!(resolve_remote_profile(&selected, &json!({}), Default::default()).is_err());
        assert!(session_override_request(&json!({"reasoningPreference":"max"}), &source["reasoning"], "responses").is_err());
    }

    #[test]
    fn shared_profile_resolution_freezes_route_and_generation_without_host_defaults() {
        let result = resolve_remote_profile(
            &profile("chat_completions"),
            &json!({"temperature":0.0,
            "generation_options":{"stop":["END"]}}),
            Default::default(),
        )
        .unwrap();
        assert_eq!(result.selected_profile.admitted_revision, 7);
        assert_eq!(result.runtime.temperature, Some(0.0));
        assert_eq!(result.runtime.top_p, Some(0.9));
        assert_eq!(result.runtime.max_output_tokens, Some(8192));
        assert_eq!(result.runtime.context_window_tokens, Some(64000));
        assert_eq!(
            result.runtime.generation_options.stop,
            Some(vec!["END".into()])
        );
        assert_eq!(
            result.runtime.model_capabilities.native_tool_calls,
            Some(true)
        );
        assert!(resolve_remote_profile(
            &profile("chat_completions"),
            &json!({"customEndpoint":"https://foreign.test/v1"}),
            Default::default()
        )
        .is_err());
        assert!(resolve_remote_profile(
            &profile("chat_completions"),
            &json!({"model":"other"}),
            Default::default()
        )
        .is_err());
    }

    #[test]
    fn profile_tiers_use_core_effort_resolution_and_freeze_selection() {
        let mut selected = profile("responses");
        let mut source: Value = serde_json::from_str(&selected.source_json).unwrap();
        source["reasoning"] = json!({"control":"effort","efforts":["low","medium","high"]});
        source["executionCapabilities"]["reasoning"] = json!(true);
        selected.source_json = source.to_string();
        for (tier, effort) in [("concise","low"),("balanced","medium"),("detailed","high")] {
            let resolved = resolve_remote_profile(&selected, &json!({"reasoning_tier":tier}), Default::default()).unwrap();
            assert_eq!(serde_json::to_value(&resolved.runtime.reasoning_selection).unwrap(),
                json!({"kind":"effort","value":effort}));
            assert_eq!(resolved.selected_profile.admitted_revision, 7);
        }
        let mut parameters: Value = serde_json::from_str(&selected.parameters_json).unwrap();
        let session = session_override_request(&json!({"reasoningTier":"concise","reasoningPreference":"automatic"}),
            &source["reasoning"], "responses").unwrap();
        assert_eq!(resolve_remote_profile(&selected, &session, Default::default()).unwrap().runtime.reasoning_selection,
            Some(ReasoningSelection::Effort {value:"low".into()}));
        let explicit = session_override_request(&json!({"reasoningTier":"concise","reasoningPreference":"high"}),
            &source["reasoning"], "responses").unwrap();
        assert!(explicit.get("reasoning_tier").is_none());
        assert_eq!(resolve_remote_profile(&selected, &explicit, Default::default()).unwrap().runtime.reasoning_selection,
            Some(ReasoningSelection::Effort {value:"high".into()}));
        parameters["reasoningTier"] = json!("detailed");
        selected.parameters_json = parameters.to_string();
        assert_eq!(resolve_remote_profile(&selected, &json!({}), Default::default()).unwrap().runtime.reasoning_selection,
            Some(ReasoningSelection::Effort {value:"high".into()}));
        assert_eq!(resolve_remote_profile(&selected, &json!({"reasoning_selection":{"kind":"effort","value":"medium"}}),
            Default::default()).unwrap().runtime.reasoning_selection,
            Some(ReasoningSelection::Effort {value:"medium".into()}));
        assert!(resolve_remote_profile(&selected, &json!({"reasoning_tier":"concise",
            "reasoning_selection":{"kind":"effort","value":"high"}}), Default::default()).is_err());
        assert!(resolve_remote_profile(&profile("responses"), &json!({"reasoning_tier":"detailed"}),
            Default::default()).unwrap().runtime.reasoning_selection.is_none());
        assert!(resolve_remote_profile(&profile("responses"), &json!({"reasoning_selection":{"kind":"effort","value":"high"}}),
            Default::default()).is_err());
    }

    #[test]
    fn sparse_session_override_preserves_profile_inheritance_and_explicit_zero() {
        let profile = profile("chat_completions");
        let request = session_override_request(
            &json!({"temperature":0.0,"topP":null}),
            &Value::Null,
            "chat_completions",
        ).unwrap();
        assert!(request.get("top_p").is_none());
        let resolved = resolve_remote_profile(&profile, &request, Default::default()).unwrap();
        assert_eq!(resolved.runtime.temperature, Some(0.0));
        assert_eq!(resolved.runtime.top_p, Some(0.9));
        assert_eq!(resolved.runtime.max_output_tokens, Some(8192));
        assert_eq!(resolved.runtime.context_window_tokens, Some(64000));
        assert_eq!(resolved.selected_profile.admitted_revision, 7);
    }

    #[test]
    fn subscription_transport_cannot_be_changed_or_receive_an_api_key() {
        let profile = profile("siwc_responses");
        let result = resolve_remote_profile(&profile, &json!({}), Default::default()).unwrap();
        assert_eq!(result.runtime.provider.as_deref(), Some("siwc_responses"));
        for request in [
            json!({"model_protocol":"responses"}),
            json!({"customApiKey":"test-only"}),
            json!({"generation_options":{"stop":["END"]}}),
        ] {
            assert!(resolve_remote_profile(&profile, &request, Default::default()).is_err());
        }
    }
}

/// Translate the current sparse conversation override contract for every Host.
pub fn session_override_request(
    value: &Value,
    reasoning: &Value,
    protocol: &str,
) -> Result<Value, String> {
    let mut request = json!({});
    for (stored, wire) in [
        ("temperature", "temperature"),
        ("topP", "top_p"),
        ("maxOutputTokens", "max_output_tokens"),
        ("maxContextLength", "max_context_tokens"),
        ("wireProtocol", "model_protocol"),
        ("samplingOverride", "sampling_override"),
    ] {
        // A nullable configuration patch clears this override. Inheritance is
        // resolved from the frozen profile, never forwarded as a JSON-null
        // physical parameter (which the typed runtime correctly rejects).
        if let Some(value) = value.get(stored).filter(|value| !value.is_null()) {
            request[wire] = value.clone();
        }
    }
    let mut options = json!({});
    for (stored, wire) in [
        ("topK", "top_k"),
        ("presencePenalty", "presence_penalty"),
        ("frequencyPenalty", "frequency_penalty"),
        ("repetitionPenalty", "repetition_penalty"),
    ] {
        if let Some(value) = value.get(stored) {
            options[wire] = value.clone();
        }
    }
    if let Some(seed) = value.get("seed").filter(|value| !value.is_null()) {
        let seed: nola_domain::generation_override::SeedOverride = serde_json::from_value(seed.clone())
            .map_err(|error| format!("invalid seed override: {error}"))?;
        options["seed"] = serde_json::to_value(seed.resolved()).map_err(|error| error.to_string())?;
    }
    if let Some(stop) = value.get("stopSequences").and_then(Value::as_str) {
        options["stop"] = json!(stop
            .lines()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>());
    }
    if options.as_object().is_some_and(|value| !value.is_empty()) {
        request["generation_options"] = options;
    }
    if let Some(tier) = value.get("reasoningTier") { request["reasoning_tier"] = tier.clone(); }
    if let Some(preference) = value.get("reasoningPreference").filter(|value| !value.is_null()) {
        if preference == "automatic" {
            request["reasoning_selection"] = Value::Null;
        } else if preference.as_str().is_some_and(|value|
            nola_domain::generation::REASONING_EFFORT_VALUES.contains(&value)) {
            let selection = json!({"kind":"effort","value":preference});
            crate::resolve_model_reasoning(reasoning,
                nola_domain::generation::ModelProtocol::parse(protocol)?, u64::MAX, true, Some(&selection))?;
            request.as_object_mut().unwrap().remove("reasoning_tier");
            request["reasoning_selection"] = selection;
        } else {
            // An exact effort preference overrides the coarse product tier.
            // Do not manufacture the mutually exclusive wire-intent pair.
            request.as_object_mut().unwrap().remove("reasoning_tier");
            let mut declaration = reasoning.clone();
            let object = declaration
                .as_object_mut()
                .ok_or("model does not declare reasoning controls")?;
            object.insert("defaultPreference".into(), preference.clone());
            request["reasoning_selection"] = serde_json::to_value(
                crate::model_reasoning_default_selection(
                    &declaration, nola_domain::generation::ModelProtocol::parse(protocol)?,
                )?,
            ).map_err(|error| format!("invalid reasoning selection: {error}"))?;
        }
    }
    Ok(request)
}

pub fn profile_with_session_contract(
    profile: &crate::ModelProfile,
    overrides: &Value,
) -> Result<crate::ModelProfile, String> {
    let mut profile = profile.clone();
    let mut source: Value =
        serde_json::from_str(&profile.source_json).map_err(|_| "invalid model source")?;
    let fields = source.as_object_mut().ok_or("model source must be an object")?;
    if let Some(value) = overrides.get("wireProtocol").filter(|value| !value.is_null()) {
        fields.insert("wireProtocol".into(), value.clone());
    }
    if let Some(value) = overrides.get("reasoning").filter(|value| !value.is_null()) {
        fields.insert("reasoning".into(), value.clone());
    }
    profile.source_json = source.to_string();
    Ok(profile)
}
