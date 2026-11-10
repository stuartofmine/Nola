// SPDX-License-Identifier: Apache-2.0
//! Shared route and parameter resolution for every Host caller, without UI or HTTP dependencies.
use serde_json::{json, Value};

#[cfg(test)]
mod session_override_tests {
    use super::*;

    #[test]
    fn cleared_scalars_inherit_profile_instead_of_sending_null_parameters() {
        let request = session_override_request(&json!({
            "temperature":null,"topP":null,"maxOutputTokens":null,
            "maxContextLength":null,"wireProtocol":null,"samplingOverride":false,
            "topK":null
        }), &json!({"control":"automatic"}), "chat_completions").unwrap();
        for field in ["temperature","top_p","max_output_tokens","max_context_tokens","model_protocol"] {
            assert!(request.get(field).is_none(), "cleared scalar {field} must inherit");
        }
        assert_eq!(request["sampling_override"], false);
        assert!(request["generation_options"]["top_k"].is_null());
        let zero = session_override_request(&json!({"temperature":0.0,"topP":0.0}),
            &json!({"control":"automatic"}), "chat_completions").unwrap();
        assert_eq!(zero["temperature"], 0.0);
        assert_eq!(zero["top_p"], 0.0);
    }

    #[test]
    fn absent_sampling_snapshot_inherits_capabilities_but_explicit_values_are_validated() {
        let mut profile = nola_application::ModelProfile {id:"sampling".into(),display_name:"Sampling".into(),
            provider:"openai-compatible".into(),model_name:"fixture".into(),runtime:"remote".into(),
            source_json:String::new(), parameters_json:json!({"temperature":0.7,"topP":0.9}).to_string(),
            selected:true,enabled:true,revision:1,created_at_ms:1,updated_at_ms:1,deleted_at_ms:None};
        for (supported, constrained) in [(false,false),(true,false),(true,true)] {
            profile.source_json=json!({"baseURL":"http://127.0.0.1:9999/v1","endpointID":"sampling",
                "wireProtocol":if constrained {"responses"} else {"chat_completions"},"capabilities":["chat"],
                "executionCapabilities":{"toolCalls":true,"imageInput":false,"samplingParameters":supported,"samplingRequiresNoReasoning":constrained,"reasoning":constrained},
                "reasoning":if constrained {json!({"control":"effort","efforts":["none","high"]})} else {Value::Null}}).to_string();
            let inherited=resolve_remote_profile_settings(&profile,&json!({}),&Default::default(),&[]).unwrap();
            assert_eq!(inherited.selected_profile, Some(nola_host_protocol::HostModelProfileReference {
                profile_id: profile.id.clone(), admitted_revision: profile.revision,
            }));
            assert_eq!(inherited.runtime.sampling_override,supported && !constrained);
            let explicit=resolve_remote_profile_settings(&profile,&json!({"temperature":0.2}),&Default::default(),&[]);
            assert_eq!(explicit.is_ok(),supported && !constrained);
            if constrained {
                let none=resolve_remote_profile_settings(&profile,&json!({"temperature":0.2,"reasoning_selection":{"kind":"effort","value":"none"}}),&Default::default(),&[]).unwrap();
                assert!(none.runtime.sampling_override);
            }
        }
    }

    #[test]
    fn siwc_profile_cannot_become_an_api_key_or_inherit_another_owner() {
        let profile = nola_application::ModelProfile {id:"fixture-siwc".into(),display_name:"Fixture".into(),
            provider:"nola-chatgpt".into(),model_name:"fixture".into(),runtime:"remote".into(),
            source_json:json!({"schemaVersion":1,"connectionID":"fixture-connection","endpointID":"fixture-connection",
                "accountID":"fixture-account","sourceKind":"nola_chatgpt","baseURL":"https://api.openai.com/v1",
                "wireProtocol":"siwc_responses","capabilities":["chat"],
                "executionCapabilities":nola_application::ModelExecutionCapabilities::default(),"reasoning":null}).to_string(),
            parameters_json:json!({"maxOutputTokens":16384,"maxContextLength":128000}).to_string(),
            selected:true,enabled:true,revision:1,created_at_ms:1,updated_at_ms:1,deleted_at_ms:None};
        let settings = resolve_remote_profile_settings(&profile, &json!({}), &Default::default(), &[]).unwrap();
        assert_eq!(settings.runtime.provider.as_deref(), Some("siwc_responses"));
        assert!(settings.api_key.is_empty());
        assert!(settings.runtime.siwc.is_none()); // trusted owner must be attached separately
        let cleared = session_override_request(&json!({"stopSequences":"", "seed":{"kind":"clear"}}),
            &Value::Null, "siwc_responses").unwrap();
        let cleared = resolve_remote_profile_settings(&profile, &cleared, &Default::default(), &[]).unwrap();
        assert_eq!(cleared.runtime.generation_options, Default::default());
        assert!(resolve_remote_profile_settings(&profile, &json!({"generation_options":{"stop":["END"]}}),
            &Default::default(), &[]).is_err());
        for request in [json!({"model_protocol":"responses"}),json!({"customApiKey":"fixture-key"}),json!({"sampling_override":true})] {
            assert!(resolve_remote_profile_settings(&profile, &request, &Default::default(), &[]).is_err());
        }
        let changed = profile_with_session_contract(&profile, &json!({"wireProtocol":"responses"})).unwrap();
        assert!(resolve_remote_profile_settings(&changed, &json!({}), &Default::default(), &[]).is_err());
    }
}

pub(crate) fn reasoning_configuration(declaration: &nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration) -> Value {
    use nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration;
    match declaration {
        ReasoningCapabilityDeclaration::Automatic {} => json!({"control":"automatic"}),
        ReasoningCapabilityDeclaration::Effort { values } => json!({"control":"effort","efforts":values}),
        ReasoningCapabilityDeclaration::Budget { tokens } => json!({"control":"budget","budgets":tokens}),
    }
}



pub(crate) use nola_application::model_profile_resolution::validate_model_request_fields;
pub(crate) use nola_application::model_profile_resolution::{
    session_override_request, profile_with_session_contract,
};

#[derive(Clone)]
pub(crate) struct ResolvedLlmSettings {
    /// Selection evidence captured from the same profile that supplied the
    /// execution settings; callers must not re-read today's selection later.
    pub(crate) selected_profile: Option<nola_host_protocol::HostModelProfileReference>,
    pub(crate) runtime: crate::runtime_config::RuntimeConfig,
    pub(crate) temperature: f64,
    pub(crate) top_p: f64,
    pub(crate) max_output_tokens: u64,
    pub(crate) max_context_tokens: u64,
    pub(crate) endpoint: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
}
impl std::fmt::Debug for ResolvedLlmSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResolvedLlmSettings { configuration: [redacted] }")
    }
}
fn trimmed_string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

pub(crate) use nola_application::model_profile_resolution::model_endpoint_identity;

pub(crate) use nola_application::model_profile_resolution::validate_remote_execution_capabilities;

pub(crate) fn validate_remote_execution_parameters(
    parameters: &Value,
    protocol: &str,
    reasoning: &Value,
    execution: nola_application::ModelExecutionCapabilities,
) -> Result<(), String> {
    validate_remote_execution_capabilities(protocol, reasoning, execution)?;
    if !execution.sampling_parameters && (parameters["temperature"].as_f64().is_some_and(|value| value != 0.7)
        || parameters["topP"].as_f64().is_some_and(|value| value != 0.9)) {
        return Err("Model declares no sampling support; reset temperature and top-p or declare support".into());
    }
    let has_penalty = parameters["presencePenalty"].as_f64().is_some_and(|value| value != 0.0)
        || parameters["frequencyPenalty"].as_f64().is_some_and(|value| value != 0.0);
    let has_seed = !parameters["seed"].is_null();
    let has_stop = parameters["stopSequences"].as_str().is_some_and(|value| !value.is_empty());
    if protocol == "responses" && (has_penalty || has_seed || has_stop) {
        return Err("Responses does not support the configured Chat Completions generation parameters".into());
    }
    if protocol == "anthropic" && (has_penalty || has_seed) {
        return Err("Anthropic does not support the configured generation parameters".into());
    }
    Ok(())
}

/// Resolve route identity, defaults and sparse request overrides together.
/// Credentials are scoped to an exact endpoint; never reuse a different route's key.
pub(crate) fn resolve_remote_profile_settings(
    profile: &nola_application::ModelProfile,
    request: &Value,
    defaults: &crate::runtime_config::RuntimeConfig,
    credentials: &[(&str, &str)],
) -> Result<ResolvedLlmSettings, String> {
    let resolved = nola_application::model_profile_resolution::resolve_remote_profile(
        profile, request, defaults.reasoning_tier.clone())?;
    let identity = model_endpoint_identity(&resolved.endpoint).ok_or("Invalid model endpoint")?;
    let api_key = trimmed_string_field(request, "customApiKey")
        .or_else(|| credentials.iter().find(|(candidate, key)| !key.is_empty() && model_endpoint_identity(candidate).as_ref() == Some(&identity)).map(|(_, key)| key.to_string()))
        .unwrap_or_default();
    let mut runtime = defaults.clone();
    runtime.siwc = None;
    runtime.llm_fallbacks.clear();
    let model = resolved.runtime;
    runtime.provider = model.provider;
    runtime.temperature = model.temperature;
    runtime.top_p = model.top_p;
    runtime.max_output_tokens = model.max_output_tokens;
    runtime.compaction.context_window_tokens = model.context_window_tokens;
    runtime.chat_output_token_field = model.chat_output_token_field;
    runtime.reasoning_selection = model.reasoning_selection;
    runtime.reasoning_tier = model.reasoning_tier;
    runtime.sampling_override = model.sampling_override;
    runtime.generation_options = model.generation_options;
    runtime.model_capabilities = model.model_capabilities;
    runtime.endpoint = Some(resolved.endpoint.clone());
    runtime.model = Some(resolved.model.clone());
    runtime.api_key = Some(api_key.clone());
    runtime.selected_profile = Some(resolved.selected_profile.clone());
    Ok(ResolvedLlmSettings {
        selected_profile: Some(resolved.selected_profile),
        endpoint: resolved.endpoint, model: resolved.model, api_key,
        temperature: runtime.temperature.unwrap_or(0.7), top_p: runtime.top_p.unwrap_or(0.9),
        max_output_tokens: runtime.max_output_tokens.unwrap_or(16384),
        max_context_tokens: runtime.compaction.context_window_tokens.unwrap_or(128000) as u64,
        runtime,
    })
}

/// Bind only after the caller has resolved the canonical owner store. Pure
/// profile parsing never opens the process default DB or reads OAuth secrets.
pub(crate) fn bind_siwc_profile(settings: &mut ResolvedLlmSettings, profile: &nola_application::ModelProfile,
    store: &crate::store::NolaStore) -> Result<(), String> {
    let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "Invalid model source")?;
    if source["wireProtocol"] != "siwc_responses" { settings.runtime.siwc = None; return Ok(()); }
    if !settings.api_key.is_empty() || source["sourceKind"] != "nola_chatgpt" {
        return Err("SIWC profile cannot use API credentials".into());
    }
    let id = source["accountID"].as_str().ok_or("SIWC account missing")?;
    let account: Option<nola_domain::model_connection::ModelAccountSnapshot> = store.application()?.execute(
        nola_application::ApplicationCommand::Model {command:nola_application::ModelCommand::GetAccount{id:id.into()}})?;
    let account = account.ok_or("SIWC account unavailable")?;
    let binding = crate::model_siwc::SiwcHost::open(store.path())?.execution_binding(id, account.revision,
        account.account.credential_ref.as_deref().ok_or("SIWC account reference missing")?)?;
    settings.runtime.siwc = Some(crate::model_siwc::SiwcExecutionContext {database:store.path().to_path_buf(),account:binding});
    settings.runtime.provider = Some("siwc_responses".into());
    settings.runtime.llm_fallbacks.clear();
    Ok(())
}
