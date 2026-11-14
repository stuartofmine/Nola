// SPDX-License-Identifier: Apache-2.0
//! Standalone CLI configuration belongs to its canonical conversation, not
//! the process environment. Native/Web adapters use their canonical settings.
use crate::{config::{Config, CommandMode}, host_invocation_request::{HostModelRequest, HostRuntimeOptions, HostExecutionRequest},
    delegated_invocation::{CredentialReference, CredentialReferenceExt, route_identity}, application_port::CliApplicationPort, store::NolaStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::{Mutex, OnceLock}};

// The read revision is retained even when configuration changes away and back.
type Baseline = (i64, Value, Value);
static BASELINES: OnceLock<Mutex<BTreeMap<String, Baseline>>> = OnceLock::new();
static APPLICATION_DEFAULTS: OnceLock<Mutex<Option<Config>>> = OnceLock::new();
fn baselines() -> &'static Mutex<BTreeMap<String, Baseline>> { BASELINES.get_or_init(Default::default) }
fn baseline_key(store: &NolaStore, id: &str) -> String { format!("{}:{id}", store.path().display()) }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationRead {
    id: String,
    revision: i64,
    configuration_revision: i64,
    settings_json: Option<String>,
}

fn read_configuration(app: &mut CliApplicationPort, id: &str) -> Result<Option<ConfigurationRead>, String> {
    let read: Option<ConfigurationRead> = app.execute(nola_application::ApplicationCommand::LoadConversationMetadata {
        conversation_id: id.into(),
    })?;
    if read.as_ref().is_some_and(|read| read.id != id || read.revision < 1 || read.configuration_revision < 1) {
        return Err("Conversation configuration identity or version mismatch".into());
    }
    Ok(read)
}
fn identity(settings: &Value) -> Value {
    json!({"executionPolicy":settings.get("executionPolicy"), "modelSelectionID":settings.get("modelSelectionID"),
        "modelOverrides":settings.get("modelOverrides"), "permissionModeRawValue":settings.get("permissionModeRawValue"),
        "memoryEnabledOverride":settings.get("memoryEnabledOverride"),
        "executionHostBinding":settings.get("executionHostBinding"),
        "executionHostRawValue":settings.get("executionHostRawValue"),
        "clientHostParameters":settings.get("clientHostParameters")})
}

fn snapshot_for_settings(settings: &Value, store: &NolaStore) -> Result<Snapshot, String> {
    if !settings["executionHostBinding"].is_null() {
        return Err("This conversation is bound to another execution Host; select a supported model before running it in this CLI".into());
    }
    let policy: Option<nola_domain::execution_policy::ConversationExecutionPolicy> =
        settings.get("executionPolicy").filter(|value| !value.is_null())
            .map(|value| serde_json::from_value(value.clone()).map_err(|error| error.to_string()))
            .transpose()?;
    let permission_profile = settings["permissionModeRawValue"].as_str()
        .and_then(crate::permissions::PermissionProfile::from_canonical_name)
        .ok_or("Conversation permission configuration missing")?;
    if let Some(policy) = policy.as_ref().filter(|policy| policy.execution_backend != "nola") {
        policy.validate()?;
        let mut runtime = crate::runtime_config::RuntimeConfig::default().host_runtime_options();
        apply_execution_policy(&mut runtime, policy);
        return Ok(Snapshot {
            model: HostModelRequest { endpoint: String::new(), api_key: String::new(),
                model: policy.external_execution_model.clone().unwrap_or_default(), swe_mode: policy.swe_mode },
            execution: execution_request(policy), runtime, credential: None,
            fallback_credentials: Vec::new(), permission_profile,
        });
    }
    if let Some(key) = settings["modelSelectionID"].as_str() {
        let id = key.strip_prefix("configured:").ok_or("This CLI executor cannot run the conversation's selected local or connected-host model")?;
        let profile = store.model_profile(id)?.ok_or("Conversation model profile is unavailable")?;
        let overrides = settings["modelOverrides"].get(key).ok_or("Conversation model parameters are missing")?;
        let profile = crate::model_profile_resolution::profile_with_session_contract(&profile, overrides)?;
        let source: Value = serde_json::from_str(&profile.source_json).map_err(|error| error.to_string())?;
        let request = crate::model_profile_resolution::session_override_request(overrides, &source["reasoning"], source["wireProtocol"].as_str().ok_or("Model source wireProtocol is missing")?)?;
        let mut resolved = crate::model_profile_resolution::resolve_remote_profile_settings(&profile, &request, &Default::default(), &[])?;
        crate::model_profile_resolution::bind_siwc_profile(&mut resolved, &profile, store)?;
        let route = route_identity(&resolved.endpoint, resolved.runtime.provider.as_deref())?;
        let credential = if let Some(context) = &resolved.runtime.siwc { CredentialReference::siwc(route, context.account.clone()) }
            else {CredentialReference::configured(route, &source)?};
        let mut runtime = resolved.runtime.host_runtime_options();
        runtime.memory_enabled = settings["memoryEnabledOverride"].as_bool();
        let mut fallback_credentials = Vec::new();
        if let Some(policy) = &policy {
            if runtime.siwc.is_some() && !policy.fallback_model_ids.is_empty() {return Err("SIWC forbids fallback billing routes".into());}
            policy.validate()?;
            apply_execution_policy(&mut runtime, policy);
            for id in &policy.fallback_model_ids {
                let profile = store.model_profile(id)?.ok_or("Saved fallback model is unavailable")?;
                let fallback = crate::model_profile_resolution::resolve_remote_profile_settings(
                    &profile, &json!({}), &Default::default(), &[],
                )?;
                let source: Value = serde_json::from_str(&profile.source_json).map_err(|error| error.to_string())?;
                fallback_credentials.push(CredentialReference::configured(
                    route_identity(&fallback.endpoint, fallback.runtime.provider.as_deref())?, &source)?);
                runtime.llm_fallbacks.push(crate::runtime_config::RuntimeLlmProvider {
                    provider: fallback.runtime.provider.clone(), endpoint: fallback.endpoint, api_key: None,
                    model: Some(fallback.model),
                });
            }
        }
        return Ok(Snapshot { model:HostModelRequest { endpoint:resolved.endpoint, model:resolved.model, api_key:String::new(), swe_mode:policy.as_ref().is_some_and(|value| value.swe_mode) },
            runtime, credential:Some(credential), fallback_credentials, permission_profile,
            execution:policy.as_ref().map(execution_request).unwrap_or(HostExecutionRequest { execution_backend:"nola".into(), external_execution_session_id:None,
                external_execution_model:None, external_execution_model_provider:None, max_iterations:0 }) });
    }
    Err("Conversation requires an explicit supported model or external execution policy".into())
}

/// Resolve a trusted captured expert configuration through the ordinary
/// conversation resolver. This is read-only: no default profile, credential,
/// conversation settings or editing baseline is created or replaced.
pub(crate) fn apply_expert_settings(
    config: &mut Config,
    settings: &Value,
    store: &NolaStore,
) -> Result<(), String> {
    restore(config, snapshot_for_settings(settings, store)?, store)
}

fn execution_request(policy: &nola_domain::execution_policy::ConversationExecutionPolicy) -> HostExecutionRequest {
    HostExecutionRequest { execution_backend: policy.execution_backend.clone(),
        external_execution_session_id: policy.external_execution_session_id.clone(),
        external_execution_model: policy.external_execution_model.clone(),
        external_execution_model_provider: policy.external_execution_model_provider.clone(),
        max_iterations: policy.max_iterations }
}

fn apply_execution_policy(runtime: &mut HostRuntimeOptions, policy: &nola_domain::execution_policy::ConversationExecutionPolicy) {
    runtime.reasoning_tier = policy.reasoning_tier;
    runtime.language = policy.language.clone();
    runtime.openai_prompt_cache = policy.openai_prompt_cache;
    runtime.openai_prompt_cache_key = policy.openai_prompt_cache_key.clone();
    runtime.openai_prompt_cache_key_prefix = policy.openai_prompt_cache_key_prefix.clone();
    runtime.openai_prompt_cache_retention = policy.openai_prompt_cache_retention.clone();
    runtime.openai_prompt_cache_history_breakpoint = policy.openai_prompt_cache_history_breakpoint;
    runtime.openai_prompt_cache_history_breakpoint_endpoints = policy.openai_prompt_cache_history_breakpoint_endpoints.clone();
    runtime.instructions = policy.instructions.clone();
    runtime.agents = policy.agents.clone();
    runtime.permissions = policy.permissions.clone();
    runtime.core_permission_rules = policy.core_permission_rules.clone();
    let model_context_window = runtime.compaction.context_window_tokens;
    runtime.compaction = policy.compaction.clone();
    // The model override is the only owner of the physical window.
    if policy.execution_backend == "nola" {
        runtime.compaction.context_window_tokens = model_context_window;
    }
    runtime.external_execution = policy.external_execution.clone();
}

fn execution_policy(config: &Config, fallback_model_ids: Vec<String>) -> Result<nola_domain::execution_policy::ConversationExecutionPolicy, String> {
    let request = config.host_invocation_request()?;
    let mut runtime = request.runtime;
    if request.execution.execution_backend == "nola" {
        runtime.compaction.context_window_tokens = None;
    }
    let policy = nola_domain::execution_policy::ConversationExecutionPolicy {
        execution_backend: request.execution.execution_backend,
        external_execution_session_id: request.execution.external_execution_session_id,
        external_execution_model: request.execution.external_execution_model,
        external_execution_model_provider: request.execution.external_execution_model_provider,
        max_iterations: request.execution.max_iterations, swe_mode: request.model_settings.swe_mode,
        reasoning_tier: runtime.reasoning_tier, language: runtime.language,
        openai_prompt_cache: runtime.openai_prompt_cache, openai_prompt_cache_key: runtime.openai_prompt_cache_key,
        openai_prompt_cache_key_prefix: runtime.openai_prompt_cache_key_prefix,
        openai_prompt_cache_retention: runtime.openai_prompt_cache_retention,
        openai_prompt_cache_history_breakpoint: runtime.openai_prompt_cache_history_breakpoint,
        openai_prompt_cache_history_breakpoint_endpoints: runtime.openai_prompt_cache_history_breakpoint_endpoints.clone(),
        instructions: runtime.instructions, agents: runtime.agents, permissions: runtime.permissions,
        core_permission_rules: runtime.core_permission_rules, compaction: runtime.compaction,
        external_execution: runtime.external_execution, fallback_model_ids,
    };
    policy.validate()?;
    Ok(policy)
}

/// Explicit CLI model input joins the same catalog as every other client.
/// Existing profiles/credentials are never overwritten by a process default.
fn ensure_remote_profile(
    store: &NolaStore, endpoint: &str, model: &str, api_key: &str,
    runtime: &HostRuntimeOptions, preferred: Option<&str>,
) -> Result<nola_application::ModelProfile, String> {
    use sha2::{Digest, Sha256};
    let route = route_identity(endpoint, runtime.provider.as_deref())?;
    let protocol = crate::llm::resolved_model_protocol(runtime.provider.as_deref(), endpoint);
    if protocol == nola_domain::generation::ModelProtocol::SiwcResponses && runtime.siwc.is_none() {
        return Err("SIWC requires an explicitly selected owned profile".into());
    }
    let matches = |profile: &nola_application::ModelProfile| {
        if profile.runtime != "remote" || profile.model_name != model || profile.deleted_at_ms.is_some() { return false; }
        serde_json::from_str::<Value>(&profile.source_json).ok().is_some_and(|source| {
            runtime.siwc.as_ref().is_none_or(|context| profile.enabled && source["accountID"] == context.account.account_id)
                && source["wireProtocol"].as_str() == Some(protocol.as_str())
                && source["baseURL"].as_str().is_some_and(|value|
                    crate::model_profile_resolution::model_endpoint_identity(value)
                        == crate::model_profile_resolution::model_endpoint_identity(endpoint))
        })
    };
    let profiles = store.model_profiles()?;
    if let Some(profile) = preferred.and_then(|id| profiles.iter().find(|profile| profile.id == id && matches(profile)))
        .or_else(|| profiles.iter().find(|profile| matches(profile))) {
        let source: Value = serde_json::from_str(&profile.source_json).map_err(|error| error.to_string())?;
        if runtime.siwc.is_none() {
            initialize_credential(store, source["endpointID"].as_str().ok_or("Model endpoint credential reference missing")?, &route, api_key)?;
        } else if !api_key.is_empty() {return Err("SIWC cannot store an API key".into());}
        return Ok(profile.clone());
    }
    if runtime.siwc.is_some() { return Err("Owned SIWC profile is unavailable; refresh and explicitly enable its catalog".into()); }
    let endpoint_id = format!("route-{:x}", Sha256::digest(route.as_bytes()));
    let profile_id = format!("model-{:x}", Sha256::digest(format!("{route}\0{model}").as_bytes()));
    let capabilities = &runtime.model_capabilities;
    let declaration = capabilities.reasoning_declaration.clone().unwrap_or(
        nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration::Automatic {});
    declaration.validate(protocol).map_err(|error| error.to_string())?;
    let reasoning = crate::model_profile_resolution::reasoning_configuration(&declaration);
    reasoning_preference(runtime.reasoning_selection.as_ref(), &reasoning, protocol)?;
    let source = json!({"endpointID":endpoint_id,"baseURL":endpoint,"wireProtocol":protocol.as_str(),
        "endpointEnabled":true,"modelEnabled":true,"capabilities":["chat","retrieval","repair"],
        "executionCapabilities":{"toolCalls":capabilities.native_tool_calls.unwrap_or(true),
            "imageInput":capabilities.vision_input.unwrap_or(false),
            "reasoning":capabilities.reasoning.unwrap_or(!matches!(declaration, nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration::Automatic {})),
            "samplingParameters":true,"chatOutputTokenField":runtime.chat_output_token_field},
        "reasoning":reasoning});
    let options = &runtime.generation_options;
    let now = chrono::Utc::now().timestamp_millis();
    let profile = nola_application::ModelProfile {
        id: profile_id.clone(), display_name: model.into(), provider: "openai-compatible".into(),
        model_name:model.into(),runtime:"remote".into(),source_json:source.to_string(),
        parameters_json:json!({"temperature":runtime.temperature.unwrap_or(0.7),
            "topP":runtime.top_p.unwrap_or(0.9),"maxOutputTokens":runtime.max_output_tokens.unwrap_or(16384),
            "maxContextLength":runtime.compaction.context_window_tokens.unwrap_or(128000),
            "topK":options.top_k,"presencePenalty":options.presence_penalty.unwrap_or(0.0),
            "frequencyPenalty":options.frequency_penalty.unwrap_or(0.0),
            "repetitionPenalty":options.repetition_penalty,
            "seed":options.seed,"stopSequences":options.stop.as_ref().map(|values| values.join("\n")).unwrap_or_default()
        }).to_string(), selected:false,enabled:true,revision:1,created_at_ms:now,updated_at_ms:now,deleted_at_ms:None,
    };
    store.upsert_model_profile(profile)?;
    let profile = store.model_profile(&profile_id)?.ok_or("Configured CLI model was not committed")?;
    initialize_credential(store, &endpoint_id, &route, api_key)?;
    Ok(profile)
}

/// Explicit initialization only. Existing current-vault credentials are never replaced.
fn initialize_credential(store: &NolaStore, endpoint_id: &str, route: &str, api_key: &str) -> Result<(), String> {
    #[cfg(any(unix, windows))]
    {
        let mut vault = crate::host_credentials::HostCredentialVault::open(store.path())?;
        if vault.record(endpoint_id)?.is_none() {
            vault.write(endpoint_id, route, 0,
                (!api_key.is_empty()).then_some(api_key), || Ok(()))?;
        }
    }
    #[cfg(not(any(unix, windows)))]
    if !api_key.is_empty() { return Err("This Host cannot persist the model credential securely".into()); }
    Ok(())
}

fn reasoning_preference(selection: Option<&nola_domain::generation::ReasoningSelection>,
    declaration: &Value, protocol: nola_domain::generation::ModelProtocol) -> Result<Value, String> {
    let Some(selection) = selection else { return Ok(json!("automatic")); };
    let candidates: Vec<&str> = if protocol == nola_domain::generation::ModelProtocol::Responses {
        match selection { nola_domain::generation::ReasoningSelection::Effort { value } => vec![value.as_str()], _ => Vec::new() }
    } else { vec!["concise", "balanced", "detailed"] };
    for candidate in candidates {
        let mut active = declaration.clone();
        active.as_object_mut().ok_or("Model has no declared thinking controls")?
            .insert("defaultPreference".into(), json!(candidate));
        if nola_application::model_reasoning_default_selection(&active, protocol).ok().flatten().as_ref() == Some(selection) {
            return Ok(json!(candidate));
        }
    }
    Err("Selected thinking control is not represented by this model's full declared capabilities; configure reasoning_declaration or select a declared tier".into())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    model: HostModelRequest,
    runtime: HostRuntimeOptions,
    execution: HostExecutionRequest,
    credential: Option<CredentialReference>,
    fallback_credentials: Vec<CredentialReference>,
    permission_profile: crate::permissions::PermissionProfile,
}

fn capture(config: &Config) -> Result<Snapshot, String> {
    let request = config.host_invocation_request()?;
    let credential = if request.execution.execution_backend == "nola" {
        let route = route_identity(&request.model_settings.endpoint, request.runtime.provider.as_deref())?;
        Some(if let Some(context) = &request.runtime.siwc {CredentialReference::siwc(route, context.account.clone())}
            else {CredentialReference::inherited(route)})
    } else { None };
    let mut model = request.model_settings;
    model.api_key.clear();
    if credential.is_none() { model.endpoint.clear(); }
    let mut runtime = request.runtime;
    let mut fallback_credentials = Vec::new();
    for route in &mut runtime.llm_fallbacks {
        fallback_credentials.push(CredentialReference::inherited(route_identity(&route.endpoint, route.provider.as_deref())?));
        route.api_key = None;
    }
    Ok(Snapshot { model, runtime, execution: request.execution, credential, fallback_credentials,
        permission_profile: request.tool_settings.permission_profile.ok_or("CLI permission profile missing")? })
}

pub(crate) fn select_profile(config: &mut Config, id: &str) -> Result<(), String> {
    let store = NolaStore::open_default()?;
    let profile = store.model_profile(id)?.ok_or("Selected Core profile is unavailable")?;
    let settings = explicit_profile_settings(&store, id, profile.revision, config)?;
    let mut updated = config.clone();
    restore(&mut updated, snapshot_for_settings(&settings, &store)?, &store)?;
    updated.cli_profile_selection = Some((id.to_string(), profile.revision));
    persist(&updated)?;
    updated.cli_profile_selection = None;
    *config = updated;
    Ok(())
}

fn explicit_profile_settings(store:&NolaStore,id:&str,revision:i64,config:&Config)->Result<Value,String> {
    let profile=store.model_profile(id)?.ok_or("Selected Core profile is unavailable")?;
    if profile.revision!=revision || !profile.enabled || profile.deleted_at_ms.is_some() || profile.runtime!="remote" {
        return Err("Selected Core profile revision or availability changed".into());
    }
    let permission=config.host_invocation_request()?.tool_settings.permission_profile.ok_or("CLI permission profile missing")?;
    let key=format!("configured:{id}");
    Ok(json!({"modelSelectionID":key,"modelOverrides":{key:{}},"permissionModeRawValue":permission.canonical_name(),"memoryEnabledOverride":config.runtime.memory_enabled}))
}

/// Called once at process/session entry, before provider exposure or recovery.
/// It never overwrites existing configuration with newly loaded global defaults.
pub(crate) fn initialize_or_restore(config: &mut Config) -> Result<(), String> {
    if config.host_request_stdin || !matches!(config.command, CommandMode::Run) || !config.save_session || config.show_help { return Ok(()); }
    let mut defaults = APPLICATION_DEFAULTS.get_or_init(Default::default).lock().map_err(|_| "CLI defaults lock poisoned")?;
    if defaults.is_none() { *defaults = Some(config.clone()); }
    drop(defaults);
    // Opening the picker is navigation, not a request to create a conversation.
    // Restore the selected identity after the user actually chooses one.
    if config.resume_session && config.session_id.is_none() { return Ok(()); }
    let store = NolaStore::open_default()?;
    let mut app = CliApplicationPort::persistent(store.path())?;
    if config.session_id.is_none() && config.continue_last {
        config.session_id = crate::session::latest_session_id_for_project(&config.project_root.to_string_lossy())?;
        config.continue_last = false;
    }
    if let Some(id) = config.session_id.as_deref() {
        if let Some(read) = read_configuration(&mut app, id)? {
            if config.cli_profile_selection.is_some() {return Err("--profile cannot replace an existing conversation's model".into());}
            let Some(raw) = read.settings_json else {
                return Err("Existing conversation configuration is missing".into());
            };
            let settings: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
            let id = id.to_string();
            restore(config, snapshot_for_settings(&settings, &store)?, &store)?;
            // Retain this object's own read witness. The editing baseline map
            // can be replaced by another caller before its tail read completes.
            config.expected_configuration_revision = Some(read.configuration_revision);
            config.expected_parent_message_id = None;
            baselines().lock().map_err(|_| "CLI baseline lock poisoned")?.insert(baseline_key(&store, &id), (read.configuration_revision, identity(&settings), serde_json::to_value(capture(config)?).map_err(|e| e.to_string())?));
            return Ok(());
        }
    }
    if config.cli_profile_selection.is_none() {
        if config.runtime.model_binding.is_some() {
            config.cli_profile_selection = Some(crate::model_connection_host::file_config::resolve_default(&store)?);
        }
    }
    if let Some((id,revision))=config.cli_profile_selection.clone() {
        let mut settings=explicit_profile_settings(&store,&id,revision,config)?;
        // A fresh model binding must preserve the current execution policy;
        // restoring model-only settings would silently discard cache preferences.
        settings["executionPolicy"] = serde_json::to_value(execution_policy(config, vec![])?).map_err(|error| error.to_string())?;
        restore(config,snapshot_for_settings(&settings,&store)?,&store)?;
    }
    let session = crate::session::SessionStore::open(config.session_id.as_deref(), false, &config.project_root.to_string_lossy())?;
    config.session_id = Some(session.id);
    persist(config)?;
    config.cli_profile_selection=None;
    Ok(())
}

/// `/new` is a fresh defaults boundary, not inheritance from the last session.
pub(crate) fn reset_new_conversation_defaults(config: &mut Config) -> Result<(), String> {
    let defaults = APPLICATION_DEFAULTS.get_or_init(Default::default).lock().map_err(|_| "CLI defaults lock poisoned")?;
    let Some(defaults) = defaults.as_ref() else { return Ok(()); };
    config.cli_profile_selection=defaults.cli_profile_selection.clone();
    config.model = defaults.model.clone();
    config.endpoint = defaults.endpoint.clone();
    config.api_key = defaults.api_key.clone();
    config.runtime = defaults.runtime.clone();
    config.permission_profile = defaults.permission_profile;
    config.auto_approve = defaults.auto_approve;
    config.network_policy = defaults.network_policy.clone();
    config.execution_backend = defaults.execution_backend.clone();
    config.external_execution_model = defaults.external_execution_model.clone();
    config.external_execution_model_provider = defaults.external_execution_model_provider.clone();
    config.external_execution_session_id = None;
    config.max_iterations = defaults.max_iterations;
    config.swe_mode = defaults.swe_mode;
    Ok(())
}

fn restore(config: &mut Config, snapshot: Snapshot, store: &NolaStore) -> Result<(), String> {
    if !snapshot.model.api_key.is_empty() || snapshot.runtime.llm_fallbacks.iter().any(|route| route.api_key.is_some()) {
        return Err("Saved CLI configuration contains credentials instead of references".into());
    }
    if snapshot.fallback_credentials.len() != snapshot.runtime.llm_fallbacks.len() { return Err("Saved fallback reference mismatch".into()); }
    let current = config.host_invocation_request()?;
    let api_key = snapshot.credential.as_ref().map(|reference| reference.resolve(&current, store)).transpose()?.unwrap_or_default();
    let mut runtime = snapshot.runtime;
    let siwc = snapshot.credential.as_ref().map(|value| value.siwc_context(store)).transpose()?.flatten();
    // Executables and environment belong to the current Host/project, not
    // the model profile. Restoring model settings must not erase these or
    // import executable paths from a different Host's snapshot.
    runtime.language_services = current.runtime.language_services.clone();
    runtime.code_mode_limits = current.runtime.code_mode_limits.clone();
    for (route, reference) in runtime.llm_fallbacks.iter_mut().zip(snapshot.fallback_credentials) {
        if route_identity(&route.endpoint, route.provider.as_deref())? != reference.route_identity { return Err("Saved fallback route mismatch".into()); }
        route.api_key = Some(reference.resolve(&current, store)?);
    }
    if let Some(reference) = &snapshot.credential {
        if route_identity(&snapshot.model.endpoint, runtime.provider.as_deref())? != reference.route_identity { return Err("Saved model route mismatch".into()); }
    }
    let mut target = serde_json::to_value(&config.runtime).map_err(|error| error.to_string())?;
    let selected_profile = runtime.selected_profile.clone();
    let frozen = serde_json::to_value(runtime).map_err(|error| error.to_string())?;
    for (key, value) in frozen.as_object().ok_or("Invalid runtime snapshot")? {
        target[key] = value.clone();
    }
    config.runtime = serde_json::from_value(target).map_err(|error| error.to_string())?;
    config.runtime.selected_profile = selected_profile;
    config.runtime.siwc = siwc;
    config.endpoint = snapshot.model.endpoint;
    config.model = snapshot.model.model;
    config.api_key = api_key;
    config.swe_mode = snapshot.model.swe_mode;
    config.runtime.endpoint = Some(config.endpoint.clone());
    config.runtime.model = Some(config.model.clone());
    config.runtime.api_key = Some(config.api_key.clone());
    config.execution_backend = snapshot.execution.execution_backend;
    config.external_execution_model = snapshot.execution.external_execution_model;
    config.external_execution_model_provider = snapshot.execution.external_execution_model_provider;
    config.external_execution_session_id = snapshot.execution.external_execution_session_id;
    config.max_iterations = snapshot.execution.max_iterations;
    config.permission_profile = Some(snapshot.permission_profile);
    config.auto_approve = snapshot.permission_profile.auto_approve();
    config.network_policy = snapshot.permission_profile.network_policy().into();
    Ok(())
}

/// Explicit interactive changes are persisted before submitting the next Run.
/// CAS prevents a second process from overwriting another owner's edit.
pub(crate) fn persist(config: &Config) -> Result<(), String> {
    use nola_domain::conversation_configuration::ConfigurationCommitResult;
    if config.host_request_stdin || !config.save_session || !matches!(config.command, CommandMode::Run) { return Ok(()); }
    let Some(id) = config.session_id.as_deref() else { return Ok(()); };
    let mut app = CliApplicationPort::persistent_default()?;
    let read = read_configuration(&mut app, id)?.ok_or("CLI conversation is unavailable")?;
    let settings: Value = read.settings_json.as_deref().map(serde_json::from_str)
        .transpose().map_err(|error| error.to_string())?.unwrap_or(json!({}));
    let captured = capture(config)?;
    let effective_permission = captured.permission_profile;
    let snapshot = serde_json::to_value(captured).map_err(|error| error.to_string())?;
    let store = NolaStore::open_default()?;
    let key = baseline_key(&store, id);
    let baseline = baselines().lock().map_err(|_| "CLI baseline lock poisoned")?.get(&key).cloned();
    if let Some((revision, previous, local)) = &baseline {
        if local == &snapshot { return Ok(()); }
        if *revision != read.configuration_revision || previous != &identity(&settings) {
            return Err("Conversation configuration changed in another client; reload before editing".into());
        }
    }
    let mut fallbacks = Vec::new();
    for route in &config.runtime.llm_fallbacks {
        let mut runtime = config.runtime.host_runtime_options();
        runtime.provider = route.provider.clone();
        runtime.llm_fallbacks.clear();
        let profile = ensure_remote_profile(&store, &route.endpoint,
            route.model.as_deref().unwrap_or(&config.model), route.api_key.as_deref().unwrap_or(""),
            &runtime, None)?;
        fallbacks.push(profile.id);
    }
    let policy = execution_policy(config, fallbacks)?;
    let patch = json!({"executionPolicy": policy,
        "permissionProfile":effective_permission.canonical_name(),
        "memoryEnabledOverride":config.runtime.memory_enabled});
    let mut selection = None;
    if config.execution_backend == "nola" {
        let request = config.host_invocation_request()?;
        if let Some((id,revision))=&config.cli_profile_selection {
            explicit_profile_settings(&store,id,*revision,config)?;
        }
        let preferred = config.cli_profile_selection.as_ref().map(|(id,_)|id.as_str())
            .or_else(||settings["modelSelectionID"].as_str().and_then(|key| key.strip_prefix("configured:")));
        let selected = ensure_remote_profile(&store, &config.endpoint, &config.model,
            &config.api_key, &request.runtime, preferred)?;
        let model_key = format!("configured:{}", selected.id);
        let source: Value = serde_json::from_str(&selected.source_json).map_err(|error| error.to_string())?;
        let protocol = nola_domain::generation::ModelProtocol::parse(source["wireProtocol"].as_str().ok_or("Model source wireProtocol is missing")?)?;
        let thinking = reasoning_preference(config.runtime.reasoning_selection.as_ref(), &source["reasoning"], protocol)?;
        let options = &config.runtime.generation_options;
        let overrides = json!({
            "reasoningPreference":thinking, "samplingOverride":config.runtime.sampling_override,
            "temperature":config.runtime.temperature, "topP":config.runtime.top_p,
            "maxOutputTokens":config.runtime.max_output_tokens,
            "maxContextLength":config.runtime.compaction.context_window_tokens,
            "topK":options.top_k,"presencePenalty":options.presence_penalty,
            "frequencyPenalty":options.frequency_penalty,"repetitionPenalty":options.repetition_penalty,
            "seed":options.seed.map(|seed| json!({"kind":"value","value":seed.to_string()})).unwrap_or(json!({"kind":"clear"})),
            "stopSequences":options.stop.as_ref().map(|values| values.join("\n")).unwrap_or_default()
        });
        // Validate the exact selection and effective parameters before saving,
        // never discover an unsupported budget only at the next restart.
        let request = crate::model_profile_resolution::session_override_request(&overrides, &source["reasoning"], protocol.as_str())?;
        crate::model_profile_resolution::resolve_remote_profile_settings(&selected, &request, &Default::default(), &[])?;
        selection = Some(nola_application::conversation_configuration::ConversationExecutionSelection::Model {
            model_id: model_key,
            parameters: overrides.as_object().ok_or("model parameters must be an object")?.clone(),
            presentation: Default::default(),
        });
    }
    let expected_revision = baseline.as_ref().map_or(read.configuration_revision, |value| value.0);
    let updated_at_ms = chrono::Utc::now().timestamp_millis();
    let command = if let Some(selection) = selection {
        nola_application::ApplicationCommand::SelectConversationExecution {
            input: nola_application::conversation_configuration::SelectConversationExecutionInput {
                conversation_id: id.into(), expected_revision, selection, updated_at_ms,
                execution_host: None, execution_backend: None, external_execution_sessions: None,
                additional_configuration: Some(patch.to_string().parse()?),
            },
        }
    } else {
        nola_application::ApplicationCommand::PatchConversationConfiguration {
            input: nola_application::conversation_configuration::PatchConversationConfigurationInput {
                conversation_id: id.into(), expected_revision,
                patch_json: patch.to_string().parse()?, updated_at_ms,
            },
        }
    };
    let result: ConfigurationCommitResult = app.execute(command)?;
    let committed = match result {
        ConfigurationCommitResult::Committed { snapshot } => snapshot,
        ConfigurationCommitResult::Conflict { .. } => return Err("Conversation configuration changed concurrently; reload before submitting".into()),
        ConfigurationCommitResult::NotFound { .. } => return Err("Conversation was deleted before its configuration could be saved".into()),
    };
    // Read the receipt's configuration, never a newer state paired with an old
    // editing revision. The metadata read only refreshes non-config identity.
    let latest = read_configuration(&mut app, id)?.ok_or("Conversation disappeared after configuration commit")?;
    if latest.configuration_revision != committed.configuration_revision {
        return Err("Conversation configuration changed after saving; reload before submitting".into());
    }
    let settings: Value = serde_json::from_str(latest.settings_json.as_deref().ok_or("Committed configuration is missing")?)
        .map_err(|error| error.to_string())?;
    baselines().lock().map_err(|_| "CLI baseline lock poisoned")?.insert(key,
        (committed.configuration_revision, identity(&settings), snapshot));
    Ok(())
}

pub(crate) fn prepare_submission(config: &Config) -> Result<Config, String> {
    let mut selected = config.clone();
    if selected.host_request_stdin || !matches!(selected.command, CommandMode::Run)
        || selected.show_help { return Ok(selected); }
    if !selected.save_session {
        if let Some(id) = selected.session_id.as_deref() {
            if let Some((revision, parent)) = crate::core_agent_runtime::ephemeral_submission_boundary(id)? {
                selected.expected_configuration_revision = Some(revision);
                selected.expected_parent_message_id = parent;
            }
        }
        return Ok(selected);
    }
    let known = match selected.session_id.as_deref() {
        Some(id) => baselines().lock().map_err(|_| "CLI baseline lock poisoned")?.contains_key(&baseline_key(&NolaStore::open_default()?, id)),
        None => false,
    };
    if !known { initialize_or_restore(&mut selected)?; }
    persist(&selected)?;
    // No local edit means canonical values win, including updates by another
    // client. Explicit edits above use the original observed baseline.
    initialize_or_restore(&mut selected)?;
    let store = NolaStore::open_default()?;
    let id = selected.session_id.as_deref().ok_or("CLI submission has no initialized conversation")?;
    let observed_revision = selected.expected_configuration_revision
        .ok_or("CLI submission has no observed configuration baseline")?;
    // The bounded tail carries configuration revision and latest visible parent
    // from one pinned snapshot. Match it to the configuration actually resolved
    // above; a concurrent edit is a conflict, never a request to refresh Begin.
    let tail: Option<nola_application::ConversationTailSnapshot> = CliApplicationPort::persistent(store.path())?.execute(
        nola_application::ApplicationCommand::LoadConversationTailSnapshotV1 {
            conversation_id: id.into(), message_limit: 1, part_text_limit: 1, part_json_limit: 1,
        },
    )?;
    freeze_prepared_submission(&mut selected, observed_revision,
        tail.as_ref().ok_or("CLI conversation was deleted during submission preparation")?)?;
    if let Some(id) = selected.session_id.as_deref() {
        // The caller may be an immutable run view. Keep its observed editing
        // baseline until the interactive owner refreshes its mutable Config.
        if let Some(baseline) = baselines().lock().map_err(|_| "CLI baseline lock poisoned")?.get_mut(&baseline_key(&NolaStore::open_default()?, id)) {
            baseline.2 = serde_json::to_value(capture(config)?).map_err(|e| e.to_string())?;
        }
    }
    Ok(selected)
}

fn freeze_prepared_submission(config: &mut Config, observed_revision: i64,
    tail: &nola_application::ConversationTailSnapshot) -> Result<(), String> {
    if config.session_id.as_deref() != Some(tail.conversation.id.as_str())
        || observed_revision < 1 || tail.conversation.configuration_revision != observed_revision {
        return Err("Conversation configuration changed while preparing input; refresh before submitting again".into());
    }
    config.expected_configuration_revision = Some(observed_revision);
    config.expected_parent_message_id = tail.conversation.messages.last().map(|message| message.id.clone());
    Ok(())
}

pub(crate) fn refresh_unmodified(config: &mut Config) -> Result<(), String> {
    let Some(id) = config.session_id.as_deref() else { return Ok(()); };
    let key = baseline_key(&NolaStore::open_default()?, id);
    let baseline = baselines().lock().map_err(|_| "CLI baseline lock poisoned")?.get(&key).cloned();
    let current = serde_json::to_value(capture(config)?).map_err(|e| e.to_string())?;
    if baseline.as_ref().is_some_and(|(_, _, local)| local == &current) {
        initialize_or_restore(config)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_explicit_profile_selection_preserves_core_identity_and_rejects_stale_revision() {
        let root=tempfile::tempdir().unwrap();let store=NolaStore::open(&root.path().join("fixture.sqlite")).unwrap();
        let config=Config::for_test().unwrap();
        let profile=ensure_remote_profile(&store,"https://fixture.invalid/v1/chat/completions","fixture","",&config.runtime.host_runtime_options(),None).unwrap();
        let settings=explicit_profile_settings(&store,&profile.id,profile.revision,&config).unwrap();
        assert_eq!(settings["modelSelectionID"],format!("configured:{}",profile.id));
        assert!(explicit_profile_settings(&store,&profile.id,profile.revision+1,&config).is_err());
        assert!(explicit_profile_settings(&store,"missing",1,&config).is_err());
        assert!(!settings.to_string().contains("api_key"));
        let snapshot=snapshot_for_settings(&settings,&store).unwrap();
        assert_eq!(snapshot.model.model,"fixture");assert!(snapshot.model.api_key.is_empty());
    }

    #[test]
    fn model_parameters_remain_authoritative_when_execution_policy_is_applied() {
        let mut runtime = crate::runtime_config::RuntimeConfig::default().host_runtime_options();
        runtime.reasoning_selection = Some(nola_domain::generation::ReasoningSelection::Effort { value: "low".into() });
        runtime.sampling_override = true;
        runtime.compaction.context_window_tokens = Some(64000);
        let mut policy = execution_policy(&Config::for_test().unwrap(), vec![]).unwrap();
        policy.reasoning_tier = nola_domain::reasoning_tiers::ReasoningTier::Detailed;
        policy.compaction.context_window_tokens = None;
        apply_execution_policy(&mut runtime, &policy);
        assert_eq!(runtime.reasoning_selection, Some(nola_domain::generation::ReasoningSelection::Effort { value: "low".into() }));
        assert!(runtime.sampling_override);
        assert_eq!(runtime.compaction.context_window_tokens, Some(64000));
        assert_eq!(runtime.reasoning_tier, nola_domain::reasoning_tiers::ReasoningTier::Detailed);
        let wire = serde_json::to_value(policy).unwrap();
        assert!(wire.get("reasoning_selection").is_none());
        assert!(wire.get("sampling_override").is_none());
    }

    #[test]
    fn thinking_selection_uses_a_complete_declaration_without_manufacturing_tiers() {
        use nola_domain::generation::{ModelProtocol, ReasoningSelection};
        let efforts = json!({"control":"effort","efforts":["low","medium","high"]});
        let budgets = json!({"control":"budget","budgets":[1024,4096,8192]});
        assert_eq!(reasoning_preference(Some(&ReasoningSelection::Effort { value:"high".into() }), &efforts, ModelProtocol::ChatCompletions).unwrap(), "detailed");
        assert_eq!(reasoning_preference(Some(&ReasoningSelection::Effort { value:"medium".into() }), &efforts, ModelProtocol::Responses).unwrap(), "medium");
        assert_eq!(reasoning_preference(Some(&ReasoningSelection::Budget { tokens:4096 }), &budgets, ModelProtocol::Anthropic).unwrap(), "balanced");
        assert!(reasoning_preference(Some(&ReasoningSelection::Budget { tokens:2048 }), &budgets, ModelProtocol::Anthropic).is_err());
        assert!(reasoning_preference(Some(&ReasoningSelection::Effort { value:"high".into() }), &json!({"control":"automatic"}), ModelProtocol::ChatCompletions).is_err());
        assert_eq!(reasoning_preference(None, &json!({"control":"automatic"}), ModelProtocol::ChatCompletions).unwrap(), "automatic");
    }

    #[test]
    fn external_executor_restores_exact_session_and_swe_mode() {
        let directory = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&directory.path().join("external.sqlite")).unwrap();
        let mut config = Config::for_test().unwrap();
        config.execution_backend = "codex".into();
        config.external_execution_session_id = Some("exact-session".into());
        config.external_execution_model = Some("external-model".into());
        config.swe_mode = true;
        let policy = execution_policy(&config, vec![]).unwrap();
        let settings = json!({"permissionModeRawValue":"cautious", "executionPolicy":policy});
        let snapshot = snapshot_for_settings(&settings, &store).unwrap();
        config.external_execution_session_id = Some("other-session".into());
        config.swe_mode = false;
        restore(&mut config, snapshot, &store).unwrap();
        assert_eq!(config.external_execution_session_id.as_deref(), Some("exact-session"));
        assert!(config.swe_mode);
    }

    #[test]
    fn canonical_model_snapshot_preserves_selected_thinking_sampling_and_swe() {
        let directory = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&directory.path().join("snapshot.sqlite")).unwrap();
        store.upsert_model_profile(nola_application::ModelProfile {
            id:"declared".into(), display_name:"Declared".into(), model_name:"fixture".into(),
            provider:"openai-compatible".into(), runtime:"remote".into(),
            source_json:json!({"baseURL":"https://fixture.invalid/v1","endpointID":"fixture-endpoint","wireProtocol":"chat_completions",
                "capabilities":["chat"],"executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true},
                "reasoning":{"control":"effort","efforts":["low","medium","high"]}}).to_string(),
            parameters_json:json!({"temperature":0.7,"topP":0.9,"maxOutputTokens":8192,"maxContextLength":64000}).to_string(),
            selected:false,enabled:true,revision:1,created_at_ms:1,updated_at_ms:1,deleted_at_ms:None,
        }).unwrap();
        let mut config = Config::for_test().unwrap();
        config.swe_mode = true;
        let policy = execution_policy(&config, vec![]).unwrap();
        let settings = json!({"permissionModeRawValue":"cautious", "executionPolicy":policy,
            "modelSelectionID":"configured:declared", "modelOverrides":{"configured:declared":{
                "reasoningPreference":"concise", "samplingOverride":true, "temperature":0.2}}});
        let snapshot = snapshot_for_settings(&settings, &store).unwrap();
        assert_eq!(snapshot.runtime.reasoning_selection, Some(nola_domain::generation::ReasoningSelection::Effort { value:"low".into() }));
        assert!(snapshot.runtime.sampling_override);
        assert!(snapshot.model.swe_mode);
        assert_eq!(snapshot.runtime.temperature, Some(0.2));
        assert!(matches!(snapshot.runtime.model_capabilities.reasoning_declaration,
            Some(nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration::Effort { ref values })
            if values == &vec!["low".to_string(), "medium".to_string(), "high".to_string()]));
    }

    #[test]
    fn submission_freezes_the_parent_only_when_it_matches_the_resolved_configuration() {
        let mut config = Config::for_test().unwrap();
        config.session_id = Some("owner".into());
        let mut source = nola_application::ConversationTailSnapshot {
            conversation: nola_application::Conversation {
                id: "owner".into(), title: "title".into(), revision: 99, configuration_revision: 7,
                created_at_ms: 1, updated_at_ms: 2, settings_json: None,
                messages: vec![nola_application::ConversationMessage { id: "observed-parent".into(), run_id: None,
                    role: "assistant".into(), status: "complete".into(), ordinal: 9, created_at_ms: 1,
                    completed_at_ms: Some(2), parts: vec![] }],
            }, total_message_count: 9, total_user_turns: 4,
        };
        freeze_prepared_submission(&mut config, 7, &source).unwrap();
        assert_eq!(config.expected_configuration_revision, Some(7));
        assert_eq!(config.expected_parent_message_id.as_deref(), Some("observed-parent"));
        source.conversation.configuration_revision = 8;
        source.conversation.messages[0].id = "new-parent".into();
        assert!(freeze_prepared_submission(&mut config, 7, &source).is_err());
        assert_eq!(config.expected_configuration_revision, Some(7));
        assert_eq!(config.expected_parent_message_id.as_deref(), Some("observed-parent"));
        source.conversation.configuration_revision = 7;
        source.conversation.id = "other-owner".into();
        assert!(freeze_prepared_submission(&mut config, 7, &source).is_err());
    }

    #[test]
    fn host_binding_cannot_fall_back_to_a_stale_cli_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&directory.path().join("host-binding.db")).unwrap();
        let settings = json!({"executionHostBinding":{"baseURL":"https://host.example"},
            "cliInvocation":{"model":"stale"}});
        let result = snapshot_for_settings(&settings, &store);
        assert!(matches!(result, Err(error) if error.contains("another execution Host")));
    }

    #[test]
    fn real_repository_restores_independent_sessions_refreshes_canonical_and_rejects_stale_edits() {
        let _environment = crate::store::test_env_lock();
        struct RestoreEnvironment(Option<std::ffi::OsString>);
        impl Drop for RestoreEnvironment {
            fn drop(&mut self) {
                match &self.0 { Some(value) => std::env::set_var("NOLA_DB_PATH", value), None => std::env::remove_var("NOLA_DB_PATH") }
            }
        }
        let _restore = RestoreEnvironment(std::env::var_os("NOLA_DB_PATH"));
        let directory = tempfile::tempdir().unwrap();
        std::env::set_var("NOLA_DB_PATH", directory.path().join("sessions.sqlite"));
        let mut first = Config::for_test().unwrap();
        first.save_session = true;
        first.session_id = Some("session-a".into());
        first.endpoint = "https://fixture.invalid/v1".into();
        first.model = "original".into();
        first.runtime.temperature = Some(0.2);
        initialize_or_restore(&mut first).unwrap();
        let mut second = first.clone();
        second.session_id = Some("session-b".into());
        second.runtime.temperature = Some(0.8);
        initialize_or_restore(&mut second).unwrap();
        let mut restarted = first.clone();
        restarted.runtime.temperature = Some(1.5);
        initialize_or_restore(&mut restarted).unwrap();
        assert_eq!(restarted.runtime.temperature, Some(0.2));
        initialize_or_restore(&mut second).unwrap();
        assert_eq!(second.runtime.temperature, Some(0.8));

        let store = NolaStore::open_default().unwrap();
        store.upsert_model_profile(nola_application::ModelProfile {
            id:"shared-profile".into(), display_name:"Shared".into(), model_name:"shared-model".into(),
            provider:"openai-compatible".into(), runtime:"remote".into(),
            source_json:json!({"baseURL":"https://fixture.invalid/v1","endpointID":"shared-endpoint","wireProtocol":"chat_completions",
                "capabilities":["chat"],"executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":true}}).to_string(),
            parameters_json:json!({"temperature":0.6,"topP":0.9,"maxOutputTokens":8192,"maxContextLength":64000}).to_string(),
            selected:true,enabled:true,revision:1,created_at_ms:1,updated_at_ms:1,deleted_at_ms:None,
        }).unwrap();
        let mut bound = first.clone();
        bound.session_id = Some("fresh-cache-policy".into());
        bound.cli_profile_selection = Some(("shared-profile".into(), 1));
        bound.runtime.openai_prompt_cache = Some(true);
        bound.runtime.openai_prompt_cache_history_breakpoint = Some(true);
        initialize_or_restore(&mut bound).unwrap();
        assert_eq!(bound.runtime.openai_prompt_cache, Some(true));
        assert_eq!(bound.runtime.openai_prompt_cache_history_breakpoint, Some(true));
        initialize_or_restore(&mut bound).unwrap();
        assert_eq!(bound.runtime.openai_prompt_cache_history_breakpoint, Some(true));
        let mut app = CliApplicationPort::persistent_default().unwrap();
        let revision = app.load_conversation("session-a").unwrap().unwrap().configuration_revision;
        let changed: nola_domain::conversation_configuration::ConfigurationCommitResult = app.execute(nola_application::ApplicationCommand::PatchConversationConfiguration {
            input:nola_application::conversation_configuration::PatchConversationConfigurationInput {
                conversation_id:"session-a".into(), expected_revision:revision,
                patch_json:json!({"modelSelectionID":"configured:shared-profile","permissionProfile":"cautious"}).to_string().parse().unwrap(),updated_at_ms:2,
            }
        }).unwrap();
        assert!(matches!(changed, nola_domain::conversation_configuration::ConfigurationCommitResult::Committed { .. }));
        let submitted = prepare_submission(&restarted).unwrap();
        let submission_source = app.load_conversation("session-a").unwrap().unwrap();
        assert_eq!(submitted.expected_configuration_revision, Some(submission_source.configuration_revision));
        assert_eq!(submitted.expected_parent_message_id, submission_source.messages.last().map(|message| message.id.clone()));
        assert_eq!(submitted.model, "shared-model");
        assert_eq!(submitted.runtime.temperature, Some(0.6));
        assert_eq!(submitted.runtime.max_output_tokens, Some(8192));
        let revision = app.load_conversation("session-a").unwrap().unwrap().configuration_revision;
        let _: nola_domain::conversation_configuration::ConfigurationCommitResult = app.execute(nola_application::ApplicationCommand::PatchConversationConfiguration {
            input:nola_application::conversation_configuration::PatchConversationConfigurationInput {
                conversation_id:"session-a".into(), expected_revision:revision,
                patch_json:json!({"modelOverrides":{"configured:shared-profile":{"temperature":0.9}}}).to_string().parse().unwrap(),updated_at_ms:3,
            }
        }).unwrap();
        restarted.runtime.temperature = Some(0.7);
        assert!(prepare_submission(&restarted).unwrap_err().contains("another client"));
        initialize_or_restore(&mut second).unwrap();
        assert_eq!(second.runtime.temperature, Some(0.8));
    }

    #[test]
    fn configuration_baseline_ignores_presentation_but_detects_shared_model_edits() {
        let before = json!({"modelSelectionID":"configured:a", "modelOverrides":{"configured:a":{"temperature":0.2}}, "title":"before"});
        let mut after = before.clone();
        after["title"] = json!("after");
        assert_eq!(identity(&before), identity(&after));
        after["modelOverrides"]["configured:a"]["temperature"] = json!(1.0);
        assert_ne!(identity(&before), identity(&after));
        after = before.clone();
        after["permissionModeRawValue"] = json!("allow_all");
        assert_ne!(identity(&before), identity(&after));
        after = before.clone();
        after["executionHostBinding"] = json!({"baseURL":"https://host.example"});
        assert_ne!(identity(&before), identity(&after));
        after = before.clone();
        after["executionHostRawValue"] = json!("remote");
        assert_ne!(identity(&before), identity(&after));
    }

    #[test]
    fn snapshot_roundtrip_keeps_generation_and_never_serializes_credentials() {
        let _environment = crate::store::test_env_lock();
        let directory = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&directory.path().join("fixture.sqlite")).unwrap();
        let mut config = Config::for_test().unwrap();
        config.endpoint = "https://fixture.invalid/v1".into();
        config.api_key = "secret-fixture-key".into();
        config.model = "original-model".into();
        config.runtime.temperature = Some(0.25);
        let encoded = serde_json::to_string(&capture(&config).unwrap()).unwrap();
        assert!(!encoded.contains("secret-fixture-key"));
        config.model = "new-global-default".into();
        config.runtime.temperature = Some(1.8);
        let service = nola_domain::language_service::LanguageServiceConfiguration {
            id:"host-service".into(), executable:"/usr/bin/clangd".into(), arguments:vec![],
            project_root:directory.path().to_path_buf(), language_ids:vec!["cpp".into()],
            environment:Default::default(), timeout_ms:1000,
        };
        config.runtime.language_services = vec![service.clone()];
        restore(&mut config, serde_json::from_str(&encoded).unwrap(), &store).unwrap();
        assert_eq!(config.model, "original-model");
        assert_eq!(config.runtime.temperature, Some(0.25));
        assert_eq!(config.runtime.language_services, vec![service]);
        assert_eq!(config.api_key, "secret-fixture-key");
        config.endpoint = "https://other.invalid/v1".into();
        assert!(restore(&mut config, serde_json::from_str(&encoded).unwrap(), &store).is_err());
    }
}
