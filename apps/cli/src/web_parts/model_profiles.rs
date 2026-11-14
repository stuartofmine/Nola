// SPDX-License-Identifier: Apache-2.0
use crate::model_profile_resolution::{model_endpoint_identity, resolve_remote_profile_settings,
    validate_remote_execution_parameters};

fn decode_run_model_intent(body: &Value) -> Result<Option<nola_application::RunGenerationIntent>, String> {
    let Some(value) = body.get("model_intent") else { return Ok(None); };
    if body.get("execution_backend").is_some_and(|backend| backend != "nola") {
        return Err("model_intent is only supported by the Nola execution backend".into());
    }
    for field in ["model", "modelProfileId", "customEndpoint", "endpoint", "customApiKey", "apiKey",
        "model_protocol", "model_capabilities", "executionCapabilities", "chat_output_token_field",
        "reasoning_selection", "reasoning_tier", "temperature", "top_p", "topP", "max_output_tokens", "maxTokens",
        "max_context_tokens", "contextWindowTokens", "sampling_override", "generation_options"] {
        if body.get(field).is_some() { return Err(format!("model_intent cannot be combined with {field}")); }
    }
    let intent: nola_application::RunGenerationIntent = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid model_intent: {error}"))?;
    intent.validate()?;
    Ok(Some(intent))
}

fn mobile_model_profile_catalog(store: &NolaStore) -> Result<Vec<Value>, String> {
    store.model_profiles()?.iter()
        .filter(|profile| profile.runtime == "remote" && profile.enabled && profile.deleted_at_ms.is_none())
        .filter_map(|profile| {
            let source: Value = match serde_json::from_str(&profile.source_json) {
                Ok(value) => value,
                Err(_) => return Some(Err("Invalid Host model profile source".into())),
            };
            if source["endpointEnabled"] == false || source["modelEnabled"] == false
                || !source["capabilities"].as_array().is_some_and(|values| values.contains(&json!("chat"))) {
                return None;
            }
            let reasoning=source.get("reasoning").and_then(Value::as_object).map(|declared| {
                let fields=["control","efforts","budgets","defaultPreference","tiers"].into_iter()
                    .filter_map(|key|declared.get(key).map(|value|(key.to_owned(),value.clone()))).collect();
                Value::Object(fields)
            });
            Some(Ok(json!({"profile_id":profile.id,"revision":profile.revision,
                "display_name":profile.display_name,"model_name":profile.model_name,"reasoning":reasoning})))
        }).collect()
}

/// This execution Host resolves its own preference. Runtime facts describe the
/// bound-profile execution path exposed by this API, not another device's runtime.
fn mobile_default_chat_model_projection(store: &NolaStore) -> Result<(Value, Option<i64>), String> {
    use nola_application::model_selection::{ChatModelCandidate, ChatModelIntent, ModelAvailability, resolve_chat_model};
    let mut application = crate::application_port::CliApplicationPort::persistent(store.path())?;
    let setting: Option<nola_application::ProductSetting> = application.execute(
        nola_application::ApplicationCommand::GetProductSetting { key: "chat.default_model".into() })?;
    let candidates = store.model_profiles()?.into_iter().filter(|profile| profile.deleted_at_ms.is_none())
        .map(|profile| {
            let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "Invalid Host model profile source")?;
            let availability = if !profile.enabled || source["endpointEnabled"] == false || source["modelEnabled"] == false {
                ModelAvailability::Disabled
            } else if profile.runtime != "remote" {
                ModelAvailability::UnsupportedRuntime
            } else { ModelAvailability::Available };
            Ok(ChatModelCandidate { profile_id: profile.id, availability,
                allowed_for_chat: source["capabilities"].as_array().is_some_and(|values| values.contains(&json!("chat"))) })
        }).collect::<Result<Vec<_>, String>>()?;
    let resolution = resolve_chat_model(&ChatModelIntent::Inherit, setting.as_ref().map(|value| value.value_json.as_str()), &candidates);
    Ok((serde_json::to_value(resolution).map_err(|error| error.to_string())?, setting.map(|value| value.revision)))
}

fn load_bound_intent_profile(
    store: &NolaStore,
    intent: &nola_application::RunGenerationIntent,
) -> Result<nola_application::ModelProfile, Response<Box<dyn Read + Send>>> {
    let profile = store.model_profile(&intent.profile_id)
        .map_err(|error| json_response(StatusCode(500), json!({"error":error})))?
        .ok_or_else(|| json_response(StatusCode(404), json!({"code":"model_profile_unavailable","error":"Selected Host model profile no longer exists"})))?;
    if profile.revision != intent.expected_revision {
        return Err(json_response(StatusCode(409), json!({"code":"model_profile_revision_conflict",
            "error":"Host model configuration changed; refresh the model catalog before sending",
            "expected_revision":intent.expected_revision,"actual_revision":profile.revision})));
    }
    Ok(profile)
}

/// A caller supplies preferences, never a replacement capability declaration.
/// Host profile identity/revision is checked before interpreting any overrides.
fn resolve_bound_model_intent_settings(
    profile: &nola_application::ModelProfile,
    intent: &nola_application::RunGenerationIntent,
    session_overrides: &Value,
    defaults: &crate::runtime_config::RuntimeConfig,
    credentials: &[(&str, &str)],
) -> Result<ResolvedLlmSettings, String> {
    intent.validate()?;
    if profile.id != intent.profile_id { return Err("model intent profile identity mismatch".into()); }
    if profile.revision != intent.expected_revision { return Err("model intent profile revision conflict".into()); }
    let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "invalid model source")?;
    let protocol = nola_domain::generation::ModelProtocol::parse(source["wireProtocol"].as_str().ok_or("model protocol is missing")?)?;
    let execution = nola_application::ModelExecutionCapabilities::from_source(&source)?;
    // Editor state may include protocol/effort metadata. The bound profile
    // owns that contract; copy only generation preferences.
    let mut preferences = json!({});
    for field in ["temperature", "topP", "maxOutputTokens", "maxContextLength", "topK",
        "presencePenalty", "frequencyPenalty", "repetitionPenalty", "seed", "stopSequences"] {
        if let Some(value) = session_overrides.get(field) { preferences[field] = value.clone(); }
    }
    let mut request = session_override_request(&preferences, &Value::Null, protocol.as_str())?;
    // Apple/Android persist editor values even when the route does not send
    // sampling parameters. Those values are not an explicit wire override.
    request["sampling_override"] = json!(execution.sampling_parameters && !execution.sampling_requires_no_reasoning);
    let mut explicit = intent.overrides.to_request()?;
    if explicit.get("sampling_override").is_none()
        && ["temperature", "top_p"].iter().any(|key| explicit.get(key).is_some()) {
        explicit["sampling_override"] = json!(true);
    }
    if let Some(options) = explicit.get_mut("generation_options").and_then(Value::as_object_mut) {
        let mut combined = request.get("generation_options").and_then(Value::as_object).cloned().unwrap_or_default();
        combined.extend(options.clone());
        *options = combined;
    }
    request.as_object_mut().unwrap().extend(explicit.as_object().ok_or("intent overrides must be an object")?.clone());
    // Core resolves the physical selection; the Host supplies product intent only.
    if intent.overrides.reasoning_selection.is_none() {
        request["reasoning_tier"] = serde_json::to_value(intent.tier).map_err(|error| error.to_string())?;
    }
    resolve_remote_profile_settings(profile, &request, defaults, credentials)
}

fn remote_profile_projection(profile: &nola_application::ModelProfile) -> Result<Value, String> {
    let source: Value = serde_json::from_str(&profile.source_json).map_err(|e| format!("invalid model source: {e}"))?;
    // Catalog discovery is not execution admission. Keep an undeclared model
    // visible for repair, without manufacturing an unsupported declaration.
    let execution = source.get("executionCapabilities")
        .filter(|value| !value.is_null())
        .map(|_| nola_application::ModelExecutionCapabilities::from_source(&source)).transpose()?;
    let execution_status = match execution {
        None => "needs_confirmation",
        Some(capabilities) if capabilities.tool_calls => "tools_enabled",
        Some(_) => "text_only",
    };
    let parameters: Value = serde_json::from_str(&profile.parameters_json).map_err(|e| format!("invalid model parameters: {e}"))?;
    // Explicit projection: do not publish arbitrary source metadata or credentials.
    Ok(json!({
        "id": profile.id, "provider": profile.provider, "displayName": profile.display_name, "modelName": profile.model_name,
        "revision": profile.revision, "selected": profile.selected,
        "enabled": source.get("modelEnabled").and_then(Value::as_bool).unwrap_or(profile.enabled),
        "endpoint": {
            "id": source.get("endpointID"), "name": source.get("endpointName"),
            "baseURL": source.get("baseURL"), "wireProtocol": source.get("wireProtocol"),
            "enabled": source.get("endpointEnabled"), "order": source.get("endpointOrder")
        },
        "priority": source.get("priority"), "order": source.get("modelOrder"),
        "capabilities": source.get("capabilities"), "reasoning": source.get("reasoning"),
        "executionCapabilities": execution,
        "executionCapabilityStatus": execution_status,
        "parameters": parameters
    }))
}

fn model_profiles_response() -> Response<Box<dyn Read + Send>> {
    match NolaStore::open_default().and_then(|store| {
        let profiles = store.model_profiles()?.iter().filter(|p| p.runtime == "remote" && p.deleted_at_ms.is_none())
            .map(remote_profile_projection).collect::<Result<Vec<_>, _>>()?;
        let (default_chat_model_resolution, _) = mobile_default_chat_model_projection(&store)?;
        Ok(json!({"profiles": profiles, "default_chat_model_resolution": default_chat_model_resolution}))
    }) {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(StatusCode(500), json!({"error": error})),
    }
}

fn create_remote_model_profile(store: &NolaStore, body: &Value) -> Result<Value, String> {
    let input = body.as_object().ok_or("model request must be an object")?;
    let expected = input.get("expectedRevisions").and_then(Value::as_object)
        .ok_or("expectedRevisions is required")?.iter().map(|(id, revision)| {
            revision.as_i64().filter(|value| *value >= 0)
                .map(|value| (id.clone(), value)).ok_or("invalid expected revision")
        }).collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    let model_name = input.get("modelName").and_then(Value::as_str).map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 512).ok_or("invalid model ID")?;
    let display_name = input.get("displayName").and_then(Value::as_str).map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 512).unwrap_or(model_name);
    let priority = input.get("priority").and_then(Value::as_u64).filter(|value| *value <= 100)
        .ok_or("priority must be an integer from 0 to 100")?;
    let capabilities = input.get("capabilities").and_then(Value::as_array).ok_or("capabilities are required")?;
    let mut uses = std::collections::BTreeSet::new();
    for capability in capabilities {
        let value = capability.as_str().filter(|value| matches!(*value, "chat" | "retrieval" | "issue_repair" | "sensitive_data"))
            .ok_or("unsupported model capability")?;
        if !uses.insert(value) { return Err("duplicate model capability".into()); }
    }
    if uses.iter().all(|value| *value == "sensitive_data") { return Err("select an allowed model use".into()); }

    let all = store.model_profiles()?;
    let mut scoped = all.into_iter().filter(|profile| profile.provider == "openai-compatible"
        && profile.runtime == "remote" && profile.deleted_at_ms.is_none()).collect::<Vec<_>>();
    let actual = scoped.iter().map(|profile| (profile.id.clone(), profile.revision))
        .collect::<std::collections::BTreeMap<_, _>>();
    if actual != expected { return Err("revision_conflict: model catalog membership or profile revision changed".into()); }
    let execution = nola_application::ModelExecutionCapabilities::from_source(body)?;
    let endpoint_id = input.get("endpointId").and_then(Value::as_str).filter(|id| !id.is_empty());
    let (endpoint_id, endpoint_name, base_url, protocol, enabled, endpoint_order, credential_account) = if let Some(id) = endpoint_id {
        let source = scoped.iter().find_map(|profile| {
            let source: Value = serde_json::from_str(&profile.source_json).ok()?;
            (source["endpointID"].as_str() == Some(id)).then_some(source)
        }).ok_or("endpoint not found")?;
        (id.to_string(), source["endpointName"].as_str().unwrap_or_default().to_string(),
            source["baseURL"].as_str().unwrap_or_default().to_string(),
            source["wireProtocol"].as_str().unwrap_or("chat_completions").to_string(),
            source["endpointEnabled"].as_bool().unwrap_or(true),
            source["endpointOrder"].as_i64().unwrap_or(0), source.get("credentialAccount").cloned())
    } else {
        let name = input.get("endpointName").and_then(Value::as_str).map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= 512).ok_or("invalid endpoint name")?;
        let url = input.get("baseURL").and_then(Value::as_str).map(str::trim)
            .filter(|value| model_endpoint_identity(value).is_some()).ok_or("invalid endpoint URL")?;
        let protocol = input.get("wireProtocol").and_then(Value::as_str)
            .filter(|value| matches!(*value, "chat_completions" | "responses" | "anthropic"))
            .ok_or("invalid API protocol")?;
        let order = scoped.iter().filter_map(|profile| serde_json::from_str::<Value>(&profile.source_json).ok())
            .filter_map(|source| source["endpointOrder"].as_i64()).max().unwrap_or(-1).saturating_add(1);
        (uuid::Uuid::new_v4().to_string(), name.to_string(), url.to_string(), protocol.to_string(), true, order, None)
    };
    if scoped.iter().any(|profile| {
        let Ok(source) = serde_json::from_str::<Value>(&profile.source_json) else { return false; };
        source["endpointID"].as_str() == Some(endpoint_id.as_str()) && profile.model_name.eq_ignore_ascii_case(model_name)
    }) { return Err("model ID already exists on this endpoint".into()); }
    let model_order = scoped.iter().filter_map(|profile| {
        let source: Value = serde_json::from_str(&profile.source_json).ok()?;
        (source["endpointID"].as_str() == Some(endpoint_id.as_str())).then(|| source["modelOrder"].as_i64().unwrap_or(0))
    }).max().unwrap_or(-1).saturating_add(1);
    let selected = !scoped.iter().any(|profile| profile.selected);
    let now = chrono::Utc::now().timestamp_millis();
    let mut source = json!({"schemaVersion":1,"endpointID":endpoint_id,"endpointName":endpoint_name,
        "baseURL":base_url,"wireProtocol":protocol,"endpointEnabled":enabled,"endpointOrder":endpoint_order,
        "modelEnabled":true,"modelOrder":model_order,"priority":priority,"capabilities":uses,
        "executionCapabilities": execution,
        "reasoning":null});
    let execution = nola_application::ModelExecutionCapabilities::from_source(&source)?;
    validate_remote_execution_parameters(&json!({"temperature":0.7,"topP":0.9}), &protocol, &source["reasoning"], execution)?;
    if let Some(account) = credential_account.filter(|value| value.as_str().is_some()) {
        source["credentialAccount"] = account;
    }
    scoped.push(nola_application::ModelProfile {
        id: uuid::Uuid::new_v4().to_string(), display_name: display_name.into(), model_name: model_name.into(),
        provider: "openai-compatible".into(), runtime: "remote".into(), source_json: source.to_string(),
        parameters_json: nola_application::remote_profile_parameters(None, None).to_string(),
        selected, enabled: true, revision: 0, created_at_ms: now, updated_at_ms: now, deleted_at_ms: None,
    });
    store.replace_remote_model_profiles(scoped, expected)?;
    Ok(json!({"profiles": store.model_profiles()?.iter().filter(|profile| profile.runtime == "remote" && profile.deleted_at_ms.is_none())
        .map(remote_profile_projection).collect::<Result<Vec<_>, _>>()?}))
}

fn create_remote_model_profile_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    match NolaStore::open_default().and_then(|store| create_remote_model_profile(&store, &body)) {
        Ok(snapshot) => json_response(StatusCode(201), snapshot),
        Err(error) => json_response(StatusCode(if error.starts_with("revision_conflict:") { 409 } else { 400 }), json!({"error": error})),
    }
}

fn mutate_remote_model_catalog(store: &NolaStore, body: &Value) -> Result<Value, String> {
    let expected = body["expectedRevisions"].as_object().ok_or("expectedRevisions is required")?
        .iter().map(|(id, value)| value.as_i64().filter(|revision| *revision >= 0)
            .map(|revision| (id.clone(), revision)).ok_or("invalid expected revision"))
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    let action = body["action"].as_str().ok_or("action is required")?;
    let target = body["targetId"].as_str().filter(|id| !id.is_empty()).ok_or("targetId is required")?;
    let mut entries = store.model_profiles()?.into_iter().filter(|profile| profile.provider == "openai-compatible"
        && profile.runtime == "remote" && profile.deleted_at_ms.is_none())
        .map(|profile| {
            let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "invalid model source")?;
            let endpoint = source["endpointID"].as_str().filter(|id| !id.is_empty()).ok_or("invalid endpoint identity")?;
            let _ = endpoint;
            Ok((profile, source))
        }).collect::<Result<Vec<_>, String>>()?;
    let actual = entries.iter().map(|(profile, _)| (profile.id.clone(), profile.revision))
        .collect::<std::collections::BTreeMap<_, _>>();
    if actual != expected { return Err("revision_conflict: model catalog membership or profile revision changed".into()); }

    let mut endpoint_ids = entries.iter().map(|(_, source)| source["endpointID"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    endpoint_ids.sort(); endpoint_ids.dedup();
    endpoint_ids.sort_by(|left, right| {
        let order = |id: &String| entries.iter().find(|(_, source)| source["endpointID"].as_str() == Some(id))
            .and_then(|(_, source)| source["endpointOrder"].as_i64()).unwrap_or(0);
        order(left).cmp(&order(right)).then_with(|| left.cmp(right))
    });
    let mut model_ids_by_endpoint = std::collections::BTreeMap::<String, Vec<String>>::new();
    for endpoint_id in &endpoint_ids {
        let mut models = entries.iter().filter(|(_, source)| source["endpointID"].as_str() == Some(endpoint_id))
            .map(|(profile, source)| (profile.id.clone(), source["modelOrder"].as_i64().unwrap_or(0)))
            .collect::<Vec<_>>();
        models.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        model_ids_by_endpoint.insert(endpoint_id.clone(), models.into_iter().map(|(id, _)| id).collect());
    }
    let direction = body["direction"].as_i64();
    match action {
        "move_endpoint" => {
            let offset = direction.filter(|value| matches!(*value, -1 | 1)).ok_or("direction must be -1 or 1")?;
            let index = endpoint_ids.iter().position(|id| id == target).ok_or("endpoint not found")?;
            let destination = index as i64 + offset;
            if destination < 0 || destination >= endpoint_ids.len() as i64 { return Err("endpoint cannot move further".into()); }
            endpoint_ids.swap(index, destination as usize);
        }
        "delete_endpoint" => {
            if !endpoint_ids.iter().any(|id| id == target) { return Err("endpoint not found".into()); }
            entries.retain(|(_, source)| source["endpointID"].as_str() != Some(target));
            endpoint_ids.retain(|id| id != target);
            model_ids_by_endpoint.remove(target);
        }
        "move_model" | "delete_model" => {
            let index = entries.iter().position(|(profile, _)| profile.id == target).ok_or("model not found")?;
            let endpoint_id = entries[index].1["endpointID"].as_str().unwrap().to_string();
            let models = model_ids_by_endpoint.get_mut(&endpoint_id).ok_or("endpoint not found")?;
            if action == "delete_model" {
                if models.len() == 1 { return Err("delete the endpoint to remove its last model".into()); }
                models.retain(|id| id != target);
                entries.remove(index);
            } else {
                let offset = direction.filter(|value| matches!(*value, -1 | 1)).ok_or("direction must be -1 or 1")?;
                let position = models.iter().position(|id| id == target).unwrap();
                let destination = position as i64 + offset;
                if destination < 0 || destination >= models.len() as i64 { return Err("model cannot move further".into()); }
                models.swap(position, destination as usize);
            }
        }
        _ => return Err("unsupported catalog action".into()),
    }
    for (endpoint_order, endpoint_id) in endpoint_ids.iter().enumerate() {
        for (model_order, model_id) in model_ids_by_endpoint.get(endpoint_id).ok_or("endpoint not found")?.iter().enumerate() {
            let index = entries.iter().position(|(profile, _)| &profile.id == model_id).ok_or("model not found")?;
            entries[index].1["endpointOrder"] = json!(endpoint_order);
            entries[index].1["modelOrder"] = json!(model_order);
        }
    }
    if !entries.is_empty() && !entries.iter().any(|(profile, _)| profile.selected) {
        let selected = entries.iter().enumerate().filter(|(_, (_, source))| source["endpointEnabled"] != false
            && source["modelEnabled"] != false && source["capabilities"].as_array().is_some_and(|uses| uses.contains(&json!("chat"))))
            .min_by_key(|(_, (profile, source))| (-source["priority"].as_i64().unwrap_or(0).clamp(0, 100),
                source["endpointOrder"].as_i64().unwrap_or(0), source["modelOrder"].as_i64().unwrap_or(0), profile.id.clone()))
            .map(|(index, _)| index).unwrap_or_else(|| entries.iter().enumerate()
                .min_by_key(|(_, (profile, source))| (source["endpointOrder"].as_i64().unwrap_or(0),
                    source["modelOrder"].as_i64().unwrap_or(0), profile.id.clone()))
                .map(|(index, _)| index).unwrap());
        entries[selected].0.selected = true;
    }
    let profiles = entries.into_iter().map(|(mut profile, source)| {
        profile.source_json = source.to_string(); profile
    }).collect::<Vec<_>>();
    store.replace_remote_model_profiles(profiles, expected)?;
    let all = store.model_profiles()?;
    let credential_cleanup_pending = if action == "delete_endpoint" && !all.iter().any(|profile| {
        profile.runtime == "remote" && profile.deleted_at_ms.is_none()
            && serde_json::from_str::<Value>(&profile.source_json).ok().and_then(|source| source["endpointID"].as_str().map(str::to_string)).as_deref() == Some(target)
    }) {
        crate::host_credentials::HostCredentialVault::open(store.path()).and_then(|mut vault| vault.purge_deleted_endpoint(target)).is_err()
    } else { false };
    let profiles = all.iter().filter(|profile| profile.runtime == "remote" && profile.deleted_at_ms.is_none())
        .map(remote_profile_projection).collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"profiles":profiles,"credentialCleanupPending":credential_cleanup_pending}))
}

fn mutate_remote_model_catalog_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    match NolaStore::open_default().and_then(|store| mutate_remote_model_catalog(&store, &body)) {
        Ok(snapshot) => json_response(StatusCode(200), snapshot),
        Err(error) => json_response(StatusCode(if error.starts_with("revision_conflict:") { 409 } else { 400 }), json!({"error":error})),
    }
}

/// Mac and Web edit the API catalog as one revisioned document. Credentials
/// stay in the host vault and are intentionally absent from this contract.
fn replace_remote_model_catalog(store: &NolaStore, body: &Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct ModelDraft {
        id: String,
        model_name: String,
        display_name: String,
        enabled: bool,
        priority: u64,
        capabilities: Vec<String>,
        execution_capabilities: Option<nola_application::ModelExecutionCapabilities>,
        reasoning: Value,
        parameters: Value,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct EndpointDraft {
        id: String,
        name: String,
        #[serde(rename = "baseURL")]
        base_url: String,
        wire_protocol: String,
        enabled: bool,
        models: Vec<ModelDraft>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct CatalogDraft {
        expected_revisions: std::collections::BTreeMap<String, i64>,
        endpoints: Vec<EndpointDraft>,
    }
    let draft: CatalogDraft = serde_json::from_value(body.clone()).map_err(|_| "invalid catalog draft")?;
    if draft.endpoints.len() > 200 { return Err("too many API endpoints".into()); }
    let all_existing = store.model_profiles()?;
    let foreign_ids = all_existing.iter().filter(|profile| profile.provider != "openai-compatible"
        || profile.runtime != "remote" || profile.deleted_at_ms.is_some())
        .map(|profile| profile.id.clone()).collect::<std::collections::HashSet<_>>();
    let existing = all_existing.into_iter().filter(|profile| profile.provider == "openai-compatible"
        && profile.runtime == "remote" && profile.deleted_at_ms.is_none()).collect::<Vec<_>>();
    let actual = existing.iter().map(|profile| (profile.id.clone(), profile.revision))
        .collect::<std::collections::BTreeMap<_, _>>();
    if actual != draft.expected_revisions { return Err("revision_conflict: model catalog membership or profile revision changed".into()); }
    let existing_by_id = existing.iter().map(|profile| (profile.id.as_str(), profile))
        .collect::<std::collections::HashMap<_, _>>();
    let mut endpoint_ids = std::collections::HashSet::new();
    let mut profile_ids = std::collections::HashSet::new();
    let mut profiles = Vec::new();
    let now = chrono::Utc::now().timestamp_millis();
    for (endpoint_order, endpoint) in draft.endpoints.iter().enumerate() {
        let id = endpoint.id.trim();
        let name = endpoint.name.trim();
        let base_url = endpoint.base_url.trim();
        if id.is_empty() || id.len() > 512 || !endpoint_ids.insert(id) || name.is_empty() || name.len() > 512
            || model_endpoint_identity(base_url).is_none() || !matches!(endpoint.wire_protocol.as_str(), "responses" | "chat_completions" | "anthropic")
            || endpoint.models.is_empty() || endpoint.models.len() > 500 {
            return Err("invalid API endpoint draft".into());
        }
        let mut model_names = std::collections::HashSet::new();
        for (model_order, model) in endpoint.models.iter().enumerate() {
            let model_id = model.id.trim();
            let model_name = model.model_name.trim();
            let display_name = model.display_name.trim();
            if model_id.is_empty() || model_id.len() > 512 || foreign_ids.contains(model_id) || !profile_ids.insert(model_id)
                || model_name.is_empty() || model_name.len() > 512 || display_name.is_empty() || display_name.len() > 512
                || !model_names.insert(model_name.to_ascii_lowercase()) || model.priority > 100 {
                return Err("invalid API model draft".into());
            }
            let capabilities = model.capabilities.iter().map(|value| value.as_str()).collect::<std::collections::BTreeSet<_>>();
            if capabilities.len() != model.capabilities.len() || capabilities.is_empty()
                || capabilities.iter().any(|value| !matches!(*value, "chat" | "retrieval" | "issue_repair" | "sensitive_data")) {
                return Err("invalid model capabilities".into());
            }
            validate_remote_parameter_patch(&model.parameters)?;
            let previous = existing_by_id.get(model_id).copied();
            if let Some(previous) = previous {
                let previous_source: Value = serde_json::from_str(&previous.source_json).map_err(|_| "invalid stored model source")?;
                if previous_source["endpointID"].as_str() != Some(id) {
                    return Err("model cannot move between endpoints".into());
                }
            }
            let mut parameters = previous.map(|profile| serde_json::from_str::<Value>(&profile.parameters_json))
                .transpose().map_err(|_| "invalid stored model parameters")?
                .unwrap_or_else(|| nola_application::remote_profile_parameters(None, None));
            let parameter_map = parameters.as_object_mut().ok_or("invalid stored model parameters")?;
            for (key, value) in model.parameters.as_object().ok_or("invalid model parameters")? {
                parameter_map.insert(key.clone(), value.clone());
            }
            let output_limit = parameters["maxOutputTokens"].as_u64().unwrap_or(16384);
            let execution = model.execution_capabilities.ok_or_else(|| format!(
                "model_execution_capabilities_required: Confirm execution capabilities for {display_name} in Model Center before saving this model"))?;
            let execution = nola_application::ModelExecutionCapabilities::from_source(
                &json!({"executionCapabilities": execution}))?;
            validate_remote_execution_parameters(&parameters, &endpoint.wire_protocol, &model.reasoning, execution)?;
            validate_profile_reasoning(&model.reasoning, &endpoint.wire_protocol, output_limit)?;
            if model.reasoning["control"].as_str().is_some_and(|control| control != "automatic")
                && !execution.reasoning {
                return Err(format!("{}: reasoning control requires declared execution support", display_name));
            }
            let mut source = previous.map(|profile| serde_json::from_str::<Value>(&profile.source_json))
                .transpose().map_err(|_| "invalid stored model source")?
                .unwrap_or_else(|| json!({"schemaVersion":1}));
            let source_map = source.as_object_mut().ok_or("invalid stored model source")?;
            for (key, value) in [
                ("endpointID", json!(id)), ("endpointName", json!(name)), ("baseURL", json!(base_url)),
                ("wireProtocol", json!(endpoint.wire_protocol)), ("endpointEnabled", json!(endpoint.enabled)),
                ("endpointOrder", json!(endpoint_order)), ("modelEnabled", json!(model.enabled)),
                ("modelOrder", json!(model_order)), ("priority", json!(model.priority)),
                ("capabilities", json!(model.capabilities)),
                ("executionCapabilities", json!(execution)),
            ] { source_map.insert(key.into(), value); }
            let mut reasoning = model.reasoning.clone();
            if let Some(reasoning) = reasoning.as_object_mut() {
                reasoning.insert("responses".into(), json!(endpoint.wire_protocol == "responses"));
                reasoning.insert("outputLimit".into(), json!(output_limit));
            }
            source_map.insert("reasoning".into(), reasoning);
            profiles.push(nola_application::ModelProfile {
                id: model_id.into(), display_name: display_name.into(), model_name: model_name.into(),
                provider: "openai-compatible".into(), runtime: "remote".into(),
                source_json: source.to_string(), parameters_json: parameters.to_string(),
                selected: false, enabled: true, revision: previous.map_or(0, |profile| profile.revision),
                created_at_ms: previous.map_or(now, |profile| profile.created_at_ms),
                updated_at_ms: now, deleted_at_ms: None,
            });
        }
    }
    if !profiles.is_empty() {
        let chosen = profiles.iter().enumerate().filter(|(_, profile)| {
            let source: Value = serde_json::from_str(&profile.source_json).unwrap_or(Value::Null);
            source["endpointEnabled"] != false && source["modelEnabled"] != false
                && source["capabilities"].as_array().is_some_and(|uses| uses.contains(&json!("chat")))
        }).min_by_key(|(_, profile)| {
            let source: Value = serde_json::from_str(&profile.source_json).unwrap_or(Value::Null);
            (-source["priority"].as_i64().unwrap_or(0), source["endpointOrder"].as_i64().unwrap_or(0),
                source["modelOrder"].as_i64().unwrap_or(0), profile.id.clone())
        }).map(|(index, _)| index).unwrap_or(0);
        profiles[chosen].selected = true;
    }
    store.replace_remote_model_profiles(profiles, draft.expected_revisions)?;
    let removed_endpoints = existing.iter().filter_map(|profile| serde_json::from_str::<Value>(&profile.source_json).ok()
        .and_then(|source| source["endpointID"].as_str().map(str::to_string)))
        .filter(|id| !endpoint_ids.contains(id.as_str())).collect::<std::collections::BTreeSet<_>>();
    let mut credential_cleanup_pending = false;
    for id in removed_endpoints {
        if crate::host_credentials::HostCredentialVault::open(store.path())
            .and_then(|mut vault| vault.purge_deleted_endpoint(&id)).is_err() {
            credential_cleanup_pending = true;
        }
    }
    let profiles = store.model_profiles()?.iter().filter(|profile| profile.runtime == "remote" && profile.deleted_at_ms.is_none())
        .map(remote_profile_projection).collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"profiles":profiles,"credentialCleanupPending":credential_cleanup_pending}))
}

fn replace_remote_model_catalog_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    match NolaStore::open_default().and_then(|store| replace_remote_model_catalog(&store, &body)) {
        Ok(snapshot) => json_response(StatusCode(200), snapshot),
        Err(error) => json_response(StatusCode(if error.starts_with("revision_conflict:") { 409 } else { 400 }), json!({"error":error})),
    }
}

fn select_remote_profile(store: &NolaStore, id: &str, revision: u64) -> Result<Value, String> {
    let mut profile = store.model_profile(id)?.ok_or("Model profile not found")?;
    let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "Invalid model source")?;
    if profile.runtime != "remote" || !profile.enabled || profile.deleted_at_ms.is_some()
        || source["endpointEnabled"] == false || source["modelEnabled"] == false
        || !source["capabilities"].as_array().is_some_and(|values| values.contains(&json!("chat"))) {
        return Err("Selected model is not available for chat".into());
    }
    profile.selected = true;
    profile.updated_at_ms = chrono::Utc::now().timestamp_millis();
    // Application's selection transaction also deselects the prior default and
    // advances its revision. Never synthesize those receipts in the browser.
    store.update_model_profile(profile, revision)?;
    let profiles = store.model_profiles()?.iter()
        .filter(|profile| profile.runtime == "remote" && profile.deleted_at_ms.is_none())
        .map(remote_profile_projection).collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"profiles": profiles}))
}

fn select_model_profile_response(request: &mut Request, id: &str) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let Some(revision) = body.get("expectedRevision").and_then(Value::as_u64) else {
        return json_response(StatusCode(400), json!({"error": "expectedRevision is required"}));
    };
    match NolaStore::open_default().and_then(|store| select_remote_profile(&store, id, revision)) {
        Ok(snapshot) => json_response(StatusCode(200), snapshot),
        Err(error) => json_response(StatusCode(409), json!({"error": error})),
    }
}

fn validate_remote_parameter_patch(parameters: &Value) -> Result<(), String> {
    let map = parameters.as_object().ok_or("parameters must be an object")?;
    for (key, value) in map {
        let valid = match key.as_str() {
            "temperature" => value.as_f64().is_some_and(|v| (0.0..=2.0).contains(&v)),
            "topP" => value.as_f64().is_some_and(|v| (0.0..=1.0).contains(&v)),
            "presencePenalty" | "frequencyPenalty" => value.as_f64().is_some_and(|v| (-2.0..=2.0).contains(&v)),
            "maxOutputTokens" => value.as_u64().is_some_and(|v| nola_domain::generation::resolve_output_token_limit(Some(v)).is_ok()),
            "maxContextLength" => value.as_u64().is_some_and(|v| (512..=4_000_000).contains(&v)),
            "seed" => value.is_null() || value.as_i64().is_some(),
            "stopSequences" => value.as_str().is_some_and(|v| v.len() <= 32768),
            _ => return Err(format!("unsupported remote parameter: {key}")),
        };
        if !valid { return Err(format!("invalid model parameter: {key}")); }
    }
    Ok(())
}

#[cfg(test)]
fn configure_remote_profile(store: &NolaStore, id: &str, revision: u64, patch: Value) -> Result<Value, String> {
    configure_remote_profile_settings(store, id, revision, patch, None)
}

fn validate_profile_reasoning(reasoning: &Value, protocol: &str, output_limit: u64) -> Result<(), String> {
    nola_application::validate_model_reasoning(reasoning, protocol, output_limit)
}

#[cfg(test)]
fn configure_remote_profile_settings(store: &NolaStore, id: &str, revision: u64, patch: Value, reasoning_patch: Option<Value>) -> Result<Value, String> {
    configure_remote_profile_full(store, id, revision, patch, reasoning_patch, None)
}

fn configure_remote_profile_full(store: &NolaStore, id: &str, revision: u64, patch: Value, reasoning_patch: Option<Value>, metadata: Option<&Value>) -> Result<Value, String> {
    validate_remote_parameter_patch(&patch)?;
    let mut profiles = store.model_profiles()?.into_iter().filter(|profile| profile.provider == "openai-compatible"
        && profile.runtime == "remote" && profile.deleted_at_ms.is_none()).collect::<Vec<_>>();
    let expected = profiles.iter().map(|profile| (profile.id.clone(), profile.revision))
        .collect::<std::collections::BTreeMap<_, _>>();
    let index = profiles.iter().position(|profile| profile.id == id).ok_or("remote model profile is unavailable")?;
    if i64::try_from(revision).ok() != Some(profiles[index].revision) { return Err("revision_conflict: model profile changed".into()); }
    let profile = &mut profiles[index];
    let mut parameters: Value = serde_json::from_str(&profile.parameters_json).map_err(|e| e.to_string())?;
    let parameters_map = parameters.as_object_mut().ok_or("invalid stored model parameters")?;
    for (key, value) in patch.as_object().unwrap() { parameters_map.insert(key.clone(), value.clone()); }
    let mut source: Value = serde_json::from_str(&profile.source_json).map_err(|e| e.to_string())?;
    if let Some(metadata) = metadata {
        if !source.is_object() { return Err("invalid model source".into()); }
        let fields = metadata.as_object().ok_or("metadata must be an object")?;
        for (key, value) in fields {
            match key.as_str() {
                "displayName" => {
                    let name = value.as_str().map(str::trim).filter(|name| !name.is_empty() && name.len() <= 512)
                        .ok_or("invalid model display name")?;
                    profile.display_name = name.into();
                }
                "enabled" => {
                    let enabled = value.as_bool().ok_or("enabled must be a boolean")?;
                    source["modelEnabled"] = json!(enabled);
                }
                "priority" => {
                    let priority = value.as_u64().filter(|value| *value <= 100).ok_or("priority must be an integer from 0 to 100")?;
                    source["priority"] = json!(priority);
                }
                "capabilities" => {
                    let capabilities = value.as_array().ok_or("capabilities must be an array")?;
                    let mut unique = std::collections::BTreeSet::new();
                    for capability in capabilities {
                        let name = capability.as_str().filter(|name| matches!(*name, "chat" | "retrieval" | "issue_repair" | "sensitive_data"))
                            .ok_or("unsupported model capability")?;
                        if !unique.insert(name) { return Err("duplicate model capability".into()); }
                    }
                    source["capabilities"] = json!(unique);
                }
                "executionCapabilities" => {
                    source["executionCapabilities"] = serde_json::to_value(serde_json::from_value::<nola_application::ModelExecutionCapabilities>(value.clone())
                        .map_err(|error| format!("invalid execution capabilities: {error}"))?).map_err(|error| error.to_string())?;
                }
                _ => return Err(format!("unsupported model metadata: {key}")),
            }
        }
    }
    let protocol = source["wireProtocol"].as_str().unwrap_or("chat_completions").to_string();
    let reasoning = reasoning_patch.as_ref().unwrap_or(&source["reasoning"]);
    let output_limit = parameters["maxOutputTokens"].as_u64().unwrap_or(16_384);
    validate_remote_execution_parameters(&parameters, &protocol, reasoning,
        nola_application::ModelExecutionCapabilities::from_source(&source)?)?;
    validate_profile_reasoning(reasoning, &protocol, output_limit)?;
    if reasoning["control"].as_str().is_some_and(|control| control != "automatic")
        && !nola_application::ModelExecutionCapabilities::from_source(&source)?.reasoning {
        return Err("reasoning control requires declared execution support".into());
    }
    if let Some(mut reasoning) = reasoning_patch {
        if let Some(object) = reasoning.as_object_mut() {
            object.insert("responses".into(), json!(protocol == "responses"));
            object.insert("outputLimit".into(), json!(output_limit));
            object.entry("efforts").or_insert(json!([]));
            object.entry("budgets").or_insert(json!([1024, 4096, 8192]));
        }
        source.as_object_mut().ok_or("invalid model source")?.insert("reasoning".into(), reasoning);
    }
    profile.source_json = serde_json::to_string(&source).map_err(|e| e.to_string())?;
    profile.parameters_json = serde_json::to_string(&parameters).map_err(|e| e.to_string())?;
    profile.updated_at_ms = chrono::Utc::now().timestamp_millis();
    // Endpoint/model enablement and the default route are one catalog fact.
    // Never leave a disabled selected model while another chat route is usable.
    let eligible = |profile: &nola_application::ModelProfile| {
        let Ok(source) = serde_json::from_str::<Value>(&profile.source_json) else { return false; };
        source["endpointEnabled"] != false && source["modelEnabled"] != false
            && source["capabilities"].as_array().is_some_and(|uses| uses.contains(&json!("chat")))
    };
    if !profiles.iter().any(|profile| profile.selected && eligible(profile)) {
        let chosen = profiles.iter().enumerate().filter(|(_, profile)| eligible(profile))
            .min_by_key(|(_, profile)| {
                let source: Value = serde_json::from_str(&profile.source_json).unwrap_or(Value::Null);
                (-source["priority"].as_i64().unwrap_or(0).clamp(0, 100),
                    source["endpointOrder"].as_i64().unwrap_or(0), source["modelOrder"].as_i64().unwrap_or(0), profile.id.clone())
            }).map(|(index, _)| index).unwrap_or(0);
        for (position, profile) in profiles.iter_mut().enumerate() { profile.selected = position == chosen; }
    }
    store.replace_remote_model_profiles(profiles, expected)?;
    let saved = store.model_profile(id)?.ok_or("saved model profile not found")?;
    remote_profile_projection(&saved)
}

fn model_endpoint_credential_response(request: &mut Request, id: &str) -> Response<Box<dyn Read + Send>> {
    use crate::host_credentials::{endpoint_binding, identity, snapshot, HostCredentialVault};
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(_) => return json_response(StatusCode(500), json!({"error":"Credential host is unavailable"})),
    };
    if request.method() == &Method::Get {
        return match snapshot(&store, id) {
            Ok(snapshot) => json_response(StatusCode(200), json!(snapshot)),
            Err(error) => json_response(StatusCode(409), json!({"error":error})),
        };
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct CredentialWrite {
        expected_revisions: std::collections::BTreeMap<String, i64>,
        #[serde(rename = "baseURL")]
        base_url: String,
        wire_protocol: String,
        credential_revision: i64,
        action: String,
        secret: Option<String>,
    }
    // Do not reuse error renderers which might quote a malformed secret/body.
    let mut bytes = Vec::new();
    if request.as_reader().take(262_145).read_to_end(&mut bytes).is_err() || bytes.len() > 262_144 {
        return json_response(StatusCode(400), json!({"error":"Invalid credential request"}));
    }
    let input = match serde_json::from_slice::<CredentialWrite>(&bytes) {
        Ok(input) => input,
        Err(_) => return json_response(StatusCode(400), json!({"error":"Invalid credential request"})),
    };
    let secret = match input.action.as_str() {
        "replace" => match input.secret.as_deref().map(str::trim) {
            Some(secret) if !secret.is_empty() && secret.len() <= 8192 && secret.bytes().all(|value| (0x21..=0x7e).contains(&value)) => Some(secret),
            _ => return json_response(StatusCode(422), json!({"error":"Credential must be a nonempty printable token of at most 8192 bytes"})),
        },
        "clear" if input.secret.is_none() => None,
        _ => return json_response(StatusCode(422), json!({"error":"Use explicit replace or clear credential action"})),
    };
    let result = (|| -> Result<_, String> {
        let bound = identity(&input.base_url, &input.wire_protocol)?;
        let validate = || -> Result<(), String> {
            let current = endpoint_binding(&store, id)?;
            if current.expected_revisions != input.expected_revisions || identity(&current.base_url, &current.wire_protocol)? != bound {
                return Err("Endpoint identity or model revisions changed; reload before retrying".into());
            }
            Ok(())
        };
        validate()?;
        HostCredentialVault::open(store.path())?.write(id, &bound, input.credential_revision, secret, validate)?;
        snapshot(&store, id)
    })();
    match result {
        Ok(snapshot) => json_response(StatusCode(200), json!(snapshot)),
        // Internal failures are deliberately opaque: never echo secret-bearing
        // decoder errors, SQL values, request bodies or a provider response.
        Err(_) => json_response(StatusCode(409), json!({"error":"Credential update was not confirmed. Reload endpoint credentials before retrying."})),
    }
}

fn model_catalog_url(base_url: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base_url).map_err(|_| "Invalid model endpoint")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("Invalid model endpoint".into());
    }
    let mut path = url.path().trim_end_matches('/').to_owned();
    for suffix in ["/chat/completions", "/responses"] {
        if let Some(base) = path.strip_suffix(suffix) { path = base.to_owned(); break; }
    }
    url.set_path(&format!("{}/models", path.trim_end_matches('/')));
    Ok(url)
}

fn parse_discovered_model_ids(body: &[u8]) -> Result<Vec<String>, String> {
    let catalog: Value = serde_json::from_slice(body).map_err(|_| "Invalid provider model catalog")?;
    let items = catalog["data"].as_array().ok_or("Invalid provider model catalog")?;
    if items.len() > 2_000 { return Err("Provider model catalog is too large".into()); }
    let mut ids = std::collections::BTreeSet::new();
    for item in items {
        if let Some(id) = item["id"].as_str().map(str::trim).filter(|id| !id.is_empty() && id.len() <= 512) {
            ids.insert(id.to_owned());
        }
    }
    Ok(ids.into_iter().collect())
}

fn discover_remote_model_ids(store: &NolaStore, endpoint_id: &str) -> Result<Vec<String>, String> {
    use crate::host_credentials::{endpoint_binding, identity, HostCredentialVault};
    let endpoint = endpoint_binding(store, endpoint_id)?;
    let bound = identity(&endpoint.base_url, &endpoint.wire_protocol)?;
    let credential = HostCredentialVault::open(store.path())?.record(endpoint_id)?;
    let secret = match credential {
        Some(record) if record.identity == bound => record.secret,
        Some(_) => return Err("Endpoint credential identity changed; reload its credentials".into()),
        None => None,
    };
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build().map_err(|_| "Cannot prepare model discovery".to_string())?;
    let mut request = client.get(model_catalog_url(&endpoint.base_url)?)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(secret) = secret { request = request.bearer_auth(secret); }
    let response = request.send().map_err(|_| "Could not reach the model catalog".to_string())?;
    if !response.status().is_success() { return Err(format!("Model discovery failed (HTTP {})", response.status().as_u16())); }
    let mut body = Vec::new();
    response.take(2_097_153).read_to_end(&mut body).map_err(|_| "Could not read the model catalog".to_string())?;
    if body.len() > 2_097_152 { return Err("Provider model catalog is too large".into()); }
    parse_discovered_model_ids(&body)
}

fn discover_remote_models_response(id: &str) -> Response<Box<dyn Read + Send>> {
    let result = NolaStore::open_default().and_then(|store| discover_remote_model_ids(&store, id));
    match result {
        Ok(models) => json_response(StatusCode(200), json!({"models":models})),
        Err(error) => json_response(StatusCode(422), json!({"error":error})),
    }
}

fn update_model_endpoint_response(request: &mut Request, id: &str) -> Response<Box<dyn Read + Send>> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct EndpointWrite {
        expected_revisions: std::collections::BTreeMap<String, i64>,
        name: String,
        #[serde(rename = "baseURL")]
        base_url: String,
        wire_protocol: String,
        enabled: bool,
    }
    let input = match read_json::<EndpointWrite>(request) { Ok(input) => input, Err(response) => return response };
    let result = NolaStore::open_default().and_then(|store| store.configure_model_endpoint(id, input.expected_revisions, input.name, input.base_url, input.wire_protocol, input.enabled))
        .and_then(|profiles| profiles.iter().filter(|profile| profile.runtime == "remote" && profile.deleted_at_ms.is_none()).map(remote_profile_projection).collect::<Result<Vec<_>, _>>());
    match result {
        Ok(profiles) => json_response(StatusCode(200), json!({"profiles": profiles})),
        Err(error) => json_response(StatusCode(409), json!({"error": error})),
    }
}

fn update_model_parameters_response(request: &mut Request, id: &str) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
    let Some(revision) = body.get("expectedRevision").and_then(Value::as_u64) else {
        return json_response(StatusCode(400), json!({"error": "expectedRevision is required"}));
    };
    let patch = body.get("parameters").cloned().unwrap_or(Value::Null);
    if let Err(error) = validate_remote_parameter_patch(&patch) {
        return json_response(StatusCode(422), json!({"error": error}));
    }
    match NolaStore::open_default().and_then(|store| {
        configure_remote_profile_full(&store, id, revision, patch, body.get("reasoning").cloned(), body.get("metadata"))?;
        let profiles = store.model_profiles()?.iter().filter(|profile| profile.runtime == "remote" && profile.deleted_at_ms.is_none())
            .map(remote_profile_projection).collect::<Result<Vec<_>, _>>()?;
        Ok(profiles)
    }) {
        Ok(profiles) => json_response(StatusCode(200), json!({"profiles": profiles})),
        Err(error) => json_response(StatusCode(409), json!({"error": error})),
    }
}

#[cfg(test)]
mod model_profile_web_tests {
    use super::*;
    use crate::runtime_config::RuntimeConfig;

    #[test]
    fn host_default_projection_uses_canonical_identity_without_fallback() {
        let (_root, store) = endpoint_fixture();
        let mut application = crate::application_port::CliApplicationPort::persistent(store.path()).unwrap();
        let _: Value = application.execute(nola_application::ApplicationCommand::PutProductSettingChecked {
            key: "chat.default_model".into(),
            value_json: json!({"version":1,"selection":{"kind":"profile","profile_id":"foreign"}}).to_string(),
            expected_revision: None, updated_at_ms: 1,
        }).unwrap();
        let (projection, revision) = mobile_default_chat_model_projection(&store).unwrap();
        assert_eq!(revision, Some(1));
        assert_eq!(projection, json!({"kind":"resolved","profile_id":"foreign","origin":"default"}));
        store.delete_model_profile("foreign", 2).unwrap();
        let (projection, revision) = mobile_default_chat_model_projection(&store).unwrap();
        assert_eq!(revision, Some(1));
        assert_eq!(projection, json!({"kind":"unavailable","profile_id":"foreign","origin":"default","reason":"missing"}));
    }

    fn endpoint_fixture() -> (tempfile::TempDir, NolaStore) {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("test.sqlite")).unwrap();
        for (id, endpoint, selected) in [("first", "endpoint", true), ("second", "endpoint", false), ("foreign", "other", false)] {
            let mut profile = route_profile();
            profile.id = id.into();
            profile.selected = selected;
            let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
            source["endpointID"] = json!(endpoint);
            source["apiKey"] = json!("fixture-secret-not-for-projection");
            profile.source_json = source.to_string();
            store.upsert_model_profile(profile).unwrap();
        }
        (root, store)
    }

    #[test]
    fn discovery_catalog_url_and_response_are_bounded() {
        assert_eq!(model_catalog_url("https://example.test/v1/chat/completions").unwrap().as_str(), "https://example.test/v1/models");
        assert_eq!(model_catalog_url("https://example.test/v1/responses").unwrap().as_str(), "https://example.test/v1/models");
        for invalid in ["file:///tmp/models", "https://user:secret@example.test/v1", "https://example.test/v1#fragment"] {
            assert!(model_catalog_url(invalid).is_err());
        }
        assert_eq!(parse_discovered_model_ids(br#"{"data":[{"id":" model-b "},{"id":"model-a"},{"id":"model-a"},{"id":""}]}"#).unwrap(), vec!["model-a", "model-b"]);
        assert!(parse_discovered_model_ids(br#"{"models":[]}"#).is_err());
        assert!(parse_discovered_model_ids(br#"{"data":"not an array"}"#).is_err());
    }

    #[test]
    fn discovery_uses_only_the_credential_bound_to_the_saved_endpoint() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1/chat/completions", server.server_addr());
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("discovery.sqlite")).unwrap();
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["endpointID"] = json!("discovery-endpoint");
        source["baseURL"] = json!(base_url);
        profile.source_json = source.to_string();
        store.upsert_model_profile(profile).unwrap();
        let binding = crate::host_credentials::identity(&base_url, "responses").unwrap();
        let mut vault = crate::host_credentials::HostCredentialVault::open(store.path()).unwrap();
        vault.write("discovery-endpoint", &binding, 0, Some("private-discovery-key"), || Ok(())).unwrap();
        let receiver = std::thread::spawn(move || {
            let request = server.recv_timeout(std::time::Duration::from_secs(10)).unwrap().expect("catalog request");
            assert_eq!(request.method(), &tiny_http::Method::Get);
            assert_eq!(request.url(), "/v1/models");
            assert!(request.headers().iter().any(|header| header.field.equiv("Authorization")
                && header.value.as_str() == "Bearer private-discovery-key"));
            request.respond(tiny_http::Response::from_string(r#"{"data":[{"id":"model-z"},{"id":"model-a"}]}"#)
                .with_header(tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap())).unwrap();
        });
        assert_eq!(discover_remote_model_ids(&store, "discovery-endpoint").unwrap(), vec!["model-a", "model-z"]);
        receiver.join().unwrap();
        vault.write("discovery-endpoint", "different-identity", 1, Some("wrong-key"), || Ok(())).unwrap();
        assert!(discover_remote_model_ids(&store, "discovery-endpoint").unwrap_err().contains("identity changed"));
    }

    fn endpoint_revisions(store: &NolaStore) -> std::collections::BTreeMap<String, i64> {
        store.model_profiles().unwrap().into_iter().filter(|profile| {
            serde_json::from_str::<Value>(&profile.source_json).unwrap()["endpointID"] == "endpoint"
        }).map(|profile| (profile.id, profile.revision)).collect()
    }

    fn catalog_revisions(store: &NolaStore) -> std::collections::BTreeMap<String, i64> {
        store.model_profiles().unwrap().into_iter().filter(|profile| profile.provider == "openai-compatible"
            && profile.runtime == "remote" && profile.deleted_at_ms.is_none())
            .map(|profile| (profile.id, profile.revision)).collect()
    }

    #[test]
    fn whole_catalog_draft_saves_order_settings_and_membership_in_one_revisioned_transaction() {
        let (_root, store) = endpoint_fixture();
        let before = catalog_revisions(&store);
        let original = store.model_profiles().unwrap();
        let model = |id: &str, priority: u64| json!({
            "id":id,"modelName":id,"displayName":id,"enabled":true,"priority":priority,
            "executionCapabilities":{"toolCalls":false,"imageInput":false,"reasoning":false,"samplingParameters":true},
            "capabilities":["chat"],"reasoning":null,"parameters":{"temperature":0.4,"maxOutputTokens":16384}
        });
        let body = json!({"expectedRevisions":before,"endpoints":[
            {"id":"endpoint","name":"Main","baseURL":"https://fixture.invalid/v1","wireProtocol":"responses",
             "enabled":true,"models":[model("second",60),model("first",80),model("new-model",90)]}
        ]});
        let invalid = json!({"expectedRevisions":before,"endpoints":[
            {"id":"endpoint","name":"Main","baseURL":"file:///tmp/invalid","wireProtocol":"responses",
             "enabled":true,"models":[model("first",80)]}
        ]});
        assert!(replace_remote_model_catalog(&store, &invalid).is_err());
        assert_eq!(store.model_profiles().unwrap(), original);
        let mut moved_identity = body.clone();
        moved_identity["endpoints"][0]["id"] = json!("different-endpoint");
        assert!(replace_remote_model_catalog(&store, &moved_identity).is_err());
        assert_eq!(store.model_profiles().unwrap(), original);
        let saved = replace_remote_model_catalog(&store, &body).unwrap();
        let profiles = saved["profiles"].as_array().unwrap();
        assert_eq!(profiles.len(), 3);
        assert_eq!(profiles.iter().find(|item| item["selected"] == true).unwrap()["id"], "new-model");
        assert_eq!(profiles.iter().find(|item| item["id"] == "second").unwrap()["order"], 0);
        assert_eq!(profiles.iter().find(|item| item["id"] == "first").unwrap()["order"], 1);
        assert_eq!(profiles.iter().find(|item| item["id"] == "new-model").unwrap()["order"], 2);
        assert_eq!(profiles.iter().find(|item| item["id"] == "first").unwrap()["parameters"]["temperature"], 0.4);
        assert!(store.model_profile("foreign").unwrap().is_none());
        assert!(replace_remote_model_catalog(&store, &body).unwrap_err().starts_with("revision_conflict:"));
    }

    #[test]
    fn catalog_moves_and_deletes_are_atomic_and_clear_deleted_endpoint_credentials() {
        let (_root, store) = endpoint_fixture();
        let identity = crate::host_credentials::identity("https://fixture.invalid/v1", "responses").unwrap();
        let mut vault = crate::host_credentials::HostCredentialVault::open(store.path()).unwrap();
        vault.write("other", &identity, 0, Some("private-key"), || Ok(())).unwrap();
        let initial = catalog_revisions(&store);
        let moved = mutate_remote_model_catalog(&store, &json!({"expectedRevisions":initial,
            "action":"move_model","targetId":"first","direction":1})).unwrap();
        assert_eq!(moved["profiles"].as_array().unwrap().iter().find(|p| p["id"] == "first").unwrap()["order"], 1);
        assert!(mutate_remote_model_catalog(&store, &json!({"expectedRevisions":initial,
            "action":"delete_model","targetId":"first"})).is_err());
        let deleted = mutate_remote_model_catalog(&store, &json!({"expectedRevisions":catalog_revisions(&store),
            "action":"delete_model","targetId":"first"})).unwrap();
        assert_eq!(deleted["profiles"].as_array().unwrap().len(), 2);
        assert_eq!(deleted["profiles"].as_array().unwrap().iter().find(|p| p["id"] == "second").unwrap()["selected"], true);
        assert!(mutate_remote_model_catalog(&store, &json!({"expectedRevisions":catalog_revisions(&store),
            "action":"delete_model","targetId":"second"})).is_err());
        let removed_endpoint = mutate_remote_model_catalog(&store, &json!({"expectedRevisions":catalog_revisions(&store),
            "action":"delete_endpoint","targetId":"other"})).unwrap();
        assert_eq!(removed_endpoint["credentialCleanupPending"], false);
        assert!(vault.record("other").unwrap().is_none());
        assert_eq!(store.model_profiles().unwrap().iter().filter(|p| p.deleted_at_ms.is_none()).count(), 1);
    }

    #[test]
    fn moving_endpoint_reindexes_every_member_without_changing_priority_or_selection() {
        let (_root, store) = endpoint_fixture();
        let original_priority = remote_profile_projection(&store.model_profile("first").unwrap().unwrap()).unwrap()["priority"].clone();
        let before = catalog_revisions(&store);
        let result = mutate_remote_model_catalog(&store, &json!({"expectedRevisions":before,
            "action":"move_endpoint","targetId":"endpoint","direction":1})).unwrap();
        for profile in result["profiles"].as_array().unwrap() {
            assert_eq!(profile["endpoint"]["order"], if profile["endpoint"]["id"] == "endpoint" { 1 } else { 0 });
        }
        let first = result["profiles"].as_array().unwrap().iter().find(|profile| profile["id"] == "first").unwrap();
        assert_eq!(first["selected"], true);
        assert_eq!(first["priority"], original_priority);
        assert!(mutate_remote_model_catalog(&store, &json!({"expectedRevisions":catalog_revisions(&store),
            "action":"move_endpoint","targetId":"endpoint","direction":1})).is_err());
    }

    #[test]
    fn creating_remote_model_requires_explicit_declaration_without_writing_or_losing_cas() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("model-declaration.sqlite")).unwrap();
        let mut request = json!({"expectedRevisions":{},"endpointName":"Research","baseURL":"https://example.test/v1",
            "wireProtocol":"chat_completions","modelName":"model-a","priority":50,"capabilities":["chat","issue_repair"]});
        assert!(create_remote_model_profile(&store, &request).unwrap_err()
            .starts_with("model_execution_capabilities_required:"));
        assert!(store.model_profiles().unwrap().is_empty());
        for invalid in [Value::Null, json!({}), json!({"toolCalls":true,"imageInput":false,"reasoning":false})] {
            request["executionCapabilities"] = invalid;
            assert!(create_remote_model_profile(&store, &request).is_err());
            assert!(store.model_profiles().unwrap().is_empty());
        }
        request["executionCapabilities"] = json!(nola_application::ModelExecutionCapabilities::default());
        let saved = create_remote_model_profile(&store, &request).unwrap();
        let profile = store.model_profile(saved["profiles"][0]["id"].as_str().unwrap()).unwrap().unwrap();
        let resolved = resolve_remote_profile_settings(&profile, &json!({}), &RuntimeConfig::default(), &[]).unwrap();
        assert_eq!(resolved.runtime.model_capabilities.native_tool_calls, Some(false));
        assert_eq!(saved["profiles"][0]["executionCapabilityStatus"], "text_only");
        let before = store.model_profiles().unwrap();
        assert!(create_remote_model_profile(&store, &request).unwrap_err().starts_with("revision_conflict:"));
        assert_eq!(store.model_profiles().unwrap(), before);
    }

    #[test]
    fn missing_catalog_declaration_cannot_change_membership_or_profile_revisions() {
        let (_root, store) = endpoint_fixture();
        let before = store.model_profiles().unwrap();
        let mut request = json!({"expectedRevisions":catalog_revisions(&store),"endpoints":[
            {"id":"endpoint","name":"Main","baseURL":"https://fixture.invalid/v1","wireProtocol":"responses",
             "enabled":true,"models":[{"id":"first","modelName":"first","displayName":"First","enabled":true,
                "priority":80,"capabilities":["chat"],"reasoning":null,"parameters":{}}]}
        ]});
        assert!(replace_remote_model_catalog(&store, &request).unwrap_err()
            .starts_with("model_execution_capabilities_required:"));
        assert_eq!(store.model_profiles().unwrap(), before);
        request["endpoints"][0]["models"][0]["executionCapabilities"] = json!(nola_application::ModelExecutionCapabilities::default());
        let saved = replace_remote_model_catalog(&store, &request).unwrap();
        assert_eq!(saved["profiles"].as_array().unwrap().len(), 1);
        assert_eq!(saved["profiles"][0]["executionCapabilityStatus"], "text_only");
        let confirmed = store.model_profiles().unwrap();
        assert!(replace_remote_model_catalog(&store, &request).unwrap_err().starts_with("revision_conflict:"));
        assert_eq!(store.model_profiles().unwrap(), confirmed);
    }

    #[test]
    fn creating_remote_models_uses_atomic_catalog_membership_and_endpoint_identity() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("models.sqlite")).unwrap();
        let request = json!({"expectedRevisions":{},"endpointName":"Research","baseURL":"https://example.test/v1",
            "wireProtocol":"responses","modelName":"model-a","displayName":"Model A","priority":70,"capabilities":["chat"],
            "executionCapabilities": nola_application::ModelExecutionCapabilities::default()});
        let first = create_remote_model_profile(&store, &request).unwrap();
        let first_profile = &first["profiles"][0];
        assert_eq!(first_profile["selected"], true);
        assert_eq!(first_profile["endpoint"]["wireProtocol"], "responses");
        assert_eq!(first_profile["parameters"]["maxContextLength"], 128000);
        assert!(create_remote_model_profile(&store, &request).is_err());
        let revisions = json!({(first_profile["id"].as_str().unwrap()):first_profile["revision"]});
        let second = create_remote_model_profile(&store, &json!({"expectedRevisions":revisions,
            "endpointId":first_profile["endpoint"]["id"],"modelName":"model-b","displayName":"Model B",
            "priority":50,"capabilities":["chat","retrieval"],
            "executionCapabilities": nola_application::ModelExecutionCapabilities::default()})).unwrap();
        assert_eq!(second["profiles"].as_array().unwrap().len(), 2);
        assert_eq!(second["profiles"][1]["endpoint"]["id"], first_profile["endpoint"]["id"]);
        assert_eq!(second["profiles"][1]["selected"], false);
        assert_eq!(second["profiles"][1]["order"], 1);
        assert!(create_remote_model_profile(&store, &json!({"expectedRevisions":{},"endpointName":"Bad",
            "baseURL":"https://example.test/#fragment","wireProtocol":"responses","modelName":"bad",
            "priority":50,"capabilities":["chat"]})).is_err());
        assert_eq!(store.model_profiles().unwrap().len(), 2);
    }

    #[test]
    fn endpoint_configuration_preserves_models_and_checks_complete_membership() {
        let (_root, store) = endpoint_fixture();
        let before = store.model_profiles().unwrap();
        let revisions = endpoint_revisions(&store);
        let after = store.configure_model_endpoint("endpoint", revisions.clone(), " Renamed ".into(), "https://new.fixture/v2?region=test".into(), "responses".into(), false).unwrap();
        for old in &before {
            let new = after.iter().find(|profile| profile.id == old.id).unwrap();
            if old.id == "foreign" { assert_eq!(new, old); continue; }
            assert_eq!(new.parameters_json, old.parameters_json);
            assert_eq!(new.selected, old.selected);
            assert_eq!(new.enabled, old.enabled);
            assert_eq!(new.model_name, old.model_name);
            let source: Value = serde_json::from_str(&new.source_json).unwrap();
            assert_eq!(source["endpointName"], "Renamed");
            assert_eq!(source["endpointEnabled"], false);
            assert_eq!(source["modelEnabled"], true);
            assert_eq!(source["apiKey"], "fixture-secret-not-for-projection");
            assert!(!remote_profile_projection(new).unwrap().to_string().contains("fixture-secret"));
        }
        assert!(store.configure_model_endpoint("endpoint", revisions, "Stale".into(), "https://new.fixture".into(), "responses".into(), true).is_err());
        let revisions = endpoint_revisions(&store);
        let mut added = store.model_profile("second").unwrap().unwrap();
        added.id = "third".into();
        store.upsert_model_profile(added).unwrap();
        let with_added = store.model_profiles().unwrap();
        assert!(store.configure_model_endpoint("endpoint", revisions, "Lost member".into(), "https://new.fixture".into(), "responses".into(), true).is_err());
        assert_eq!(store.model_profiles().unwrap(), with_added);
    }

    #[test]
    fn endpoint_configuration_rejects_invalid_urls_and_incompatible_reasoning_atomically() {
        let (_root, store) = endpoint_fixture();
        let before = store.model_profiles().unwrap();
        for url in ["file:///tmp/test", "https://user:secret@fixture.invalid", "https://fixture.invalid/#fragment", "not a url"] {
            assert!(store.configure_model_endpoint("endpoint", endpoint_revisions(&store), "Name".into(), url.into(), "responses".into(), true).is_err());
            assert_eq!(store.model_profiles().unwrap(), before);
        }
        for protocol in ["anthropic", "chat_completions", "unknown"] {
            assert!(store.configure_model_endpoint("endpoint", endpoint_revisions(&store), "Name".into(), "https://fixture.invalid".into(), protocol.into(), true).is_err());
            assert_eq!(store.model_profiles().unwrap(), before);
        }
        let mut revisions = endpoint_revisions(&store);
        revisions.remove("second");
        assert!(store.configure_model_endpoint("endpoint", revisions, "Name".into(), "https://fixture.invalid".into(), "responses".into(), true).is_err());
        assert_eq!(store.model_profiles().unwrap(), before);
    }

    #[test]
    fn endpoint_configuration_rolls_back_first_model_when_later_write_fails() {
        let (root, store) = endpoint_fixture();
        let before = store.model_profiles().unwrap();
        let connection = rusqlite::Connection::open(root.path().join("test.sqlite")).unwrap();
        let outbox_before: i64 = connection.query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0)).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_second_endpoint_write BEFORE UPDATE ON model_profiles WHEN NEW.id='second' BEGIN SELECT RAISE(ABORT, 'fixture endpoint write failure'); END;").unwrap();
        assert!(store.configure_model_endpoint("endpoint", endpoint_revisions(&store), "Name".into(), "https://fixture.invalid".into(), "responses".into(), false).is_err());
        assert_eq!(store.model_profiles().unwrap(), before);
        let outbox_after: i64 = connection.query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0)).unwrap();
        assert_eq!(outbox_after, outbox_before, "Rolled back endpoint edits must not publish sync records");
    }

    fn route_profile() -> nola_application::ModelProfile {
        nola_application::ModelProfile {
            id:"profile".into(), display_name:"Fixture".into(), model_name:"fixture-model".into(),
            provider:"openai-compatible".into(), runtime:"remote".into(),
            source_json:json!({"baseURL":"https://fixture.invalid/v1", "wireProtocol":"responses",
                "endpointEnabled":true,"modelEnabled":true,"capabilities":["chat"],
                "executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true},
                "reasoning":{"control":"effort","efforts":["low","high"]}}).to_string(),
            parameters_json:json!({"temperature":0.7,"topP":0.9,"maxOutputTokens":64000,"maxContextLength":4000000}).to_string(),
            selected:true, enabled:true, revision:1, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        }
    }

    #[test]
    fn expert_room_exact_effort_reaches_bound_runtime_without_becoming_a_tier() {
        let mut profile = route_profile();
        for effort in ["low", "high"] {
            let settings = json!({"modelOverrides":{"configured:profile":{"reasoningPreference":effort}}});
            let intent = room_generation_intent(&profile, &settings).unwrap();
            let resolved = resolve_bound_model_intent_settings(&profile, &intent, &Value::Null,
                &crate::runtime_config::RuntimeConfig::default(), &[]).unwrap();
            assert_eq!(serde_json::to_value(resolved.runtime.reasoning_selection).unwrap(), json!({"kind":"effort","value":effort}));
        }
        assert!(room_generation_intent(&profile, &json!({"modelOverrides":{"configured:profile":{"reasoningPreference":"max"}}})).is_err());
        profile.parameters_json = json!({"reasoningPreference":"high"}).to_string();
        let intent = room_generation_intent(&profile, &json!({})).unwrap();
        assert_eq!(serde_json::to_value(intent.overrides.reasoning_selection).unwrap(), json!({"kind":"effort","value":"high"}));
        let inherited = room_generation_intent(&profile, &json!({"modelOverrides":{"configured:profile":{"reasoningPreference":"automatic"}}})).unwrap();
        assert!(inherited.overrides.reasoning_selection.is_none());
    }

    #[test]
    fn bound_intent_keeps_native_editor_sampling_values_off_an_unsupported_wire() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["executionCapabilities"]["samplingParameters"] = json!(false);
        profile.source_json = source.to_string();
        let preferences = json!({"temperature":0.7,"topP":0.9});
        let resolved = resolve_bound_model_intent_settings(&profile, &intent(&profile,json!({})),
            &preferences, &crate::runtime_config::RuntimeConfig::default(), &[]).unwrap();
        assert!(!resolved.runtime.sampling_override);
        // An explicit wire request still requires declared support; never silently drop it.
        assert!(resolve_bound_model_intent_settings(&profile, &intent(&profile,json!({"temperature":0.7})),
            &preferences, &crate::runtime_config::RuntimeConfig::default(), &[]).is_err());
    }

    fn intent(profile: &nola_application::ModelProfile, overrides: Value) -> nola_application::RunGenerationIntent {
        decode_run_model_intent(&json!({"model_intent":{"profile_id":profile.id,
            "expected_revision":profile.revision,"tier":"balanced","overrides":overrides}})).unwrap().unwrap()
    }

    #[test]
    fn host_model_intent_preserves_tier_when_wire_efforts_are_identical() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["reasoning"]["efforts"] = json!(["low"]);
        profile.source_json = source.to_string();
        for tier in [nola_domain::reasoning_tiers::ReasoningTier::Concise,
            nola_domain::reasoning_tiers::ReasoningTier::Balanced,
            nola_domain::reasoning_tiers::ReasoningTier::Detailed] {
            let mut requested = intent(&profile, json!({}));
            requested.tier = tier;
            let resolved = resolve_bound_model_intent_settings(&profile, &requested, &Value::Null,
                &RuntimeConfig::default(), &[]).unwrap();
            assert_eq!(resolved.runtime.reasoning_tier, tier);
            assert_eq!(resolved.runtime.reasoning_selection, Some(nola_domain::generation::ReasoningSelection::Effort { value:"low".into() }));
        }
        assert!(decode_run_model_intent(&json!({"model_intent":{"profile_id":profile.id,
            "expected_revision":profile.revision,"tier":"balanced","overrides":{}},
            "reasoning_tier":"detailed"})).is_err());
    }

    #[test]
    fn host_model_intent_preserves_zero_clear_and_session_precedence() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["wireProtocol"] = json!("chat_completions");
        source["reasoning"]["efforts"] = json!(["low","medium","high"]);
        profile.source_json = source.to_string();
        let mut parameters: Value = serde_json::from_str(&profile.parameters_json).unwrap();
        parameters["seed"] = json!(42);
        parameters["presencePenalty"] = json!(0.5);
        profile.parameters_json = parameters.to_string();
        let requested = intent(&profile, json!({"temperature":0,"generation_options":{"seed":0,"presence_penalty":null}}));
        let session = json!({"temperature":0.9,"topP":0.4,"seed":{"kind":"value","value":"7"},"presencePenalty":0.8,
            "wireProtocol":"anthropic","reasoning":{"control":"budget","budgets":[1024,2048,4096]}});
        let resolved = resolve_bound_model_intent_settings(&profile, &requested, &session, &RuntimeConfig::default(), &[]).unwrap();
        assert_eq!(resolved.temperature, 0.0);
        assert_eq!(resolved.top_p, 0.4);
        assert_eq!(resolved.runtime.generation_options.seed, Some(0));
        assert_eq!(resolved.runtime.generation_options.presence_penalty, None);
        assert_eq!(resolved.runtime.reasoning_selection, Some(nola_domain::generation::ReasoningSelection::Effort { value:"medium".into() }));
        assert_eq!(resolved.runtime.provider.as_deref(), Some("openai-compatible"));
        assert_eq!(resolved.model, "fixture-model");
    }

    #[test]
    fn host_model_intent_accepts_real_created_responses_defaults() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("created.sqlite")).unwrap();
        let catalog = create_remote_model_profile(&store, &json!({"expectedRevisions":{},
            "endpointName":"Responses","baseURL":"https://fixture.invalid/v1",
            "wireProtocol":"responses","modelName":"created-model","displayName":"Created",
            "priority":50,"capabilities":["chat"],
            "executionCapabilities": nola_application::ModelExecutionCapabilities::default()})).unwrap();
        let id = catalog["profiles"][0]["id"].as_str().unwrap();
        let profile = store.model_profile(id).unwrap().unwrap();
        let parameters: Value = serde_json::from_str(&profile.parameters_json).unwrap();
        assert!(parameters["topK"].is_null());
        assert!(parameters["repetitionPenalty"].is_null());
        let requested = intent(&profile, json!({}));
        let bound = load_bound_intent_profile(&store, &requested).ok().unwrap();
        let resolved = resolve_bound_model_intent_settings(&bound, &requested, &Value::Null,
            &RuntimeConfig::default(), &[]).unwrap();
        assert_eq!(resolved.runtime.provider.as_deref(), Some("openai-responses"));
        assert_eq!(resolved.runtime.generation_options, Default::default());
        assert_eq!(resolved.runtime.reasoning_selection, None);
    }

    #[test]
    fn host_model_intent_rejects_client_authority_and_never_falls_back() {
        let profile = route_profile();
        let body = json!({"model_intent":{"profile_id":profile.id,"expected_revision":profile.revision,"tier":"balanced","overrides":{}}});
        for field in ["model", "modelProfileId", "endpoint", "customEndpoint", "apiKey", "customApiKey",
            "model_protocol", "executionCapabilities", "reasoning_selection", "model_capabilities", "temperature"] {
            let mut invalid = body.clone();
            invalid[field] = Value::Null;
            assert!(decode_run_model_intent(&invalid).is_err(), "accepted conflicting {field}");
        }
        for field in ["model", "apiKey", "endpoint", "model_protocol", "executionCapabilities", "reasoning_selection"] {
            let mut invalid = body.clone();
            invalid["model_intent"]["overrides"][field] = json!("injected");
            assert!(decode_run_model_intent(&invalid).is_err());
        }
        assert!(decode_run_model_intent(&json!({"model_intent":null})).is_err());
        let mut external = body.clone();
        external["execution_backend"] = json!("codex");
        assert!(decode_run_model_intent(&external).is_err());
        let mut unavailable = profile.clone();
        unavailable.enabled = false;
        assert!(resolve_bound_model_intent_settings(&unavailable, &intent(&profile, json!({})), &Value::Null, &RuntimeConfig::default(), &[]).is_err());
        let mut unsupported = profile.clone();
        let mut source: Value = serde_json::from_str(&unsupported.source_json).unwrap();
        source["executionCapabilities"]["reasoning"] = json!(false);
        unsupported.source_json = source.to_string();
        assert!(resolve_bound_model_intent_settings(&unsupported, &intent(&profile, json!({})), &Value::Null, &RuntimeConfig::default(), &[]).is_err());
    }

    #[test]
    fn host_model_intent_explicit_clear_shadows_incompatible_profile_default() {
        let mut profile = route_profile();
        let mut parameters: Value = serde_json::from_str(&profile.parameters_json).unwrap();
        parameters["seed"] = json!(42);
        profile.parameters_json = parameters.to_string();
        assert!(resolve_bound_model_intent_settings(&profile, &intent(&profile, json!({})), &Value::Null, &RuntimeConfig::default(), &[]).is_err());
        let clear = intent(&profile, json!({"generation_options":{"seed":null}}));
        let resolved = resolve_bound_model_intent_settings(&profile, &clear, &Value::Null, &RuntimeConfig::default(), &[]).unwrap();
        assert_eq!(resolved.runtime.generation_options.seed, None);
        assert_eq!(resolved.runtime.provider.as_deref(), Some("openai-responses"));
        let session_clear = resolve_bound_model_intent_settings(&profile, &intent(&profile, json!({})),
            &json!({"seed":{"kind":"clear"}}), &RuntimeConfig::default(), &[]).unwrap();
        assert_eq!(session_clear.runtime.generation_options.seed, None);
        let source: Value = serde_json::from_str(&profile.source_json).unwrap();
        assert!(validate_remote_execution_parameters(&parameters, "responses", &source["reasoning"],
            nola_application::ModelExecutionCapabilities::from_source(&source).unwrap()).is_err());
    }

    #[test]
    fn host_model_intent_resolves_session_seed_intent_without_losing_i64_precision() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["wireProtocol"] = json!("chat_completions");
        source["reasoning"]["efforts"] = json!(["low","medium","high"]);
        profile.source_json = source.to_string();
        let mut parameters: Value = serde_json::from_str(&profile.parameters_json).unwrap();
        parameters["seed"] = json!(42);
        profile.parameters_json = parameters.to_string();
        let requested = intent(&profile, json!({}));
        let mut cases = vec![(json!({}), Some(42)), (json!({"seed":null}), Some(42)),
            (json!({"seed":{"kind":"clear"}}), None)];
        for seed in [0, -1, 9_007_199_254_740_993, i64::MIN, i64::MAX] {
            cases.push((json!({"seed":{"kind":"value","value":seed.to_string()}}), Some(seed)));
        }
        for (session, expected) in cases {
            let resolved = resolve_bound_model_intent_settings(&profile, &requested, &session,
                &RuntimeConfig::default(), &[]).unwrap();
            assert_eq!(resolved.runtime.generation_options.seed, expected, "{session}");
        }
        for seed in [json!(0), json!({"kind":"value","value":"01"})] {
            assert!(resolve_bound_model_intent_settings(&profile, &requested, &json!({"seed":seed}),
                &RuntimeConfig::default(), &[]).is_err());
        }
    }

    #[test]
    fn host_model_intent_revision_conflict_and_catalog_are_read_only() {
        let (_root, store) = endpoint_fixture();
        let profile = store.model_profile("first").unwrap().unwrap();
        let observer = rusqlite::Connection::open(store.path()).unwrap();
        let version = || observer.query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0)).unwrap();
        let before = version();
        let mut requested = intent(&profile, json!({}));
        requested.expected_revision += 1;
        assert_eq!(load_bound_intent_profile(&store, &requested).err().unwrap().status_code(), StatusCode(409));
        requested.profile_id = profile.model_name.clone();
        assert_eq!(load_bound_intent_profile(&store, &requested).err().unwrap().status_code(), StatusCode(404));
        let profiles = mobile_model_profile_catalog(&store).unwrap();
        assert_eq!(profiles.len(), 3);
        for item in profiles {
            assert_eq!(item.as_object().unwrap().len(), 4);
            assert!(item.get("profile_id").is_some());
            assert!(item.get("revision").is_some());
            assert!(!item.to_string().contains("fixture-secret"));
        }
        assert_eq!(version(), before);
    }

    #[test]
    fn chat_output_limit_field_is_frozen_from_exact_model_profile() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["wireProtocol"] = json!("chat_completions");
        source["reasoning"] = Value::Null;
        source["executionCapabilities"]["chatOutputTokenField"] = json!("max_completion_tokens");
        profile.source_json = source.to_string();
        let mut defaults = RuntimeConfig::default();
        defaults.chat_output_token_field = nola_application::ChatOutputTokenField::MaxTokens;
        let resolved = resolve_remote_profile_settings(&profile, &json!({}), &defaults, &[]).unwrap();
        assert_eq!(resolved.runtime.chat_output_token_field,
            nola_application::ChatOutputTokenField::MaxCompletionTokens);
    }

    #[test]
    fn protocol_override_is_validated_before_resolving_generation() {
        let mut profile = route_profile();
        let defaults = RuntimeConfig::default();
        for protocol in [Value::Null, json!("unknown"), json!(1)] {
            assert!(resolve_remote_profile_settings(&profile, &json!({"model_protocol": protocol}), &defaults, &[]).is_err());
        }
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["wireProtocol"] = json!("chat_completions");
        source["reasoning"] = Value::Null;
        profile.source_json = source.to_string();
        profile.parameters_json = json!({"seed":42}).to_string();
        assert!(resolve_remote_profile_settings(&profile,
            &json!({"model_protocol":"responses"}), &defaults, &[])
            .unwrap_err().contains("Responses does not support"));
        profile.parameters_json = json!({}).to_string();
        let resolved = resolve_remote_profile_settings(&profile,
            &json!({"model_protocol":"responses"}), &defaults, &[]).unwrap();
        assert_eq!(resolved.runtime.provider.as_deref(), Some("openai-responses"));
        assert_eq!(resolved.runtime.model_capabilities.reasoning_continuity, Some(true));
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["wireProtocol"] = json!("responses");
        profile.source_json = source.to_string();
        let resolved = resolve_remote_profile_settings(&profile,
            &json!({"model_protocol":"chat_completions", "generation_options":{"seed":0}}), &defaults, &[]).unwrap();
        assert_eq!(resolved.runtime.provider.as_deref(), Some("openai-compatible"));
        assert_eq!(resolved.runtime.generation_options.seed, Some(0));
        assert_eq!(resolved.runtime.model_capabilities.reasoning_continuity, Some(false));
        assert!(defaults.provider.is_none());
    }

    #[test]
    fn sampling_restriction_requires_explicit_none_effort() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["executionCapabilities"]["samplingRequiresNoReasoning"] = json!(true);
        source["reasoning"]["efforts"] = json!(["none", "low", "high"]);
        profile.source_json = source.to_string();
        let defaults = RuntimeConfig::default();
        let automatic = resolve_remote_profile_settings(&profile, &json!({}), &defaults, &[]).unwrap();
        assert!(!automatic.runtime.sampling_override, "automatic effort must keep provider sampling defaults");
        assert!(resolve_remote_profile_settings(&profile,
            &json!({"sampling_override":true,"reasoning_selection":{"kind":"effort","value":"high"}}), &defaults, &[])
            .unwrap_err().contains("explicit reasoning effort none"));
        assert!(resolve_remote_profile_settings(&profile,
            &json!({"sampling_override":true,"reasoning_selection":{"kind":"effort","value":"none"}}), &defaults, &[]).is_ok());
    }

    #[test]
    fn undeclared_execution_capabilities_fail_closed_before_model_invocation() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source.as_object_mut().unwrap().remove("executionCapabilities");
        profile.source_json = source.to_string();
        for request in [json!({}), json!({"temperature":0.7}),
            json!({"reasoning_selection":{"kind":"effort","value":"high"}})] {
            assert!(resolve_remote_profile_settings(&profile, &request, &RuntimeConfig::default(), &[])
                .unwrap_err().starts_with("model_execution_capabilities_required:"));
        }
        let projected = remote_profile_projection(&profile).unwrap();
        assert_eq!(projected["id"], profile.id);
        assert!(projected["executionCapabilities"].is_null());
        assert_eq!(projected["executionCapabilityStatus"], "needs_confirmation");
        source["executionCapabilities"] = Value::Null;
        profile.source_json = source.to_string();
        assert!(resolve_remote_profile_settings(&profile, &json!({}), &RuntimeConfig::default(), &[])
            .unwrap_err().starts_with("model_execution_capabilities_required:"));
        let projected_null = remote_profile_projection(&profile).unwrap();
        assert!(projected_null["executionCapabilities"].is_null());
        assert_eq!(projected_null["executionCapabilityStatus"], "needs_confirmation");
    }

    #[test]
    fn profile_rejects_generation_options_unsupported_by_its_protocol() {
        let profile = route_profile();
        assert!(resolve_remote_profile_settings(&profile, &json!({"generation_options":{"seed":4}}), &RuntimeConfig::default(), &[])
            .unwrap_err().contains("Responses does not support"));
        let mut anthropic = profile.clone();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["wireProtocol"] = json!("anthropic");
        source["reasoning"] = json!({"control":"budget","budgets":[1024,4096,8192]});
        anthropic.source_json = source.to_string();
        assert!(resolve_remote_profile_settings(&anthropic, &json!({"generation_options":{"frequency_penalty":0.4}}), &RuntimeConfig::default(), &[])
            .unwrap_err().contains("Anthropic does not support"));
        assert!(resolve_remote_profile_settings(&anthropic, &json!({"reasoning_selection":{"kind":"budget","tokens":2048}}), &RuntimeConfig::default(), &[])
            .unwrap_err().contains("not declared"));
    }

    #[test]
    fn automatic_control_does_not_activate_stale_effort_metadata() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["reasoning"]["control"] = json!("automatic");
        profile.source_json = source.to_string();
        assert!(resolve_remote_profile_settings(&profile, &json!({}), &RuntimeConfig::default(), &[]).is_ok());
        assert!(resolve_remote_profile_settings(&profile,
            &json!({"reasoning_selection":{"kind":"effort","value":"low"}}),
            &RuntimeConfig::default(), &[]).unwrap_err().contains("not declared"));
    }

    #[test]
    fn remote_default_selection_returns_canonical_revisions_and_rejects_stale_edits() {
        let root = std::env::temp_dir().join(format!("nola-web-model-selection-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        fs::create_dir_all(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
        let _cleanup = Cleanup(root.clone());
        let store = NolaStore::open(&root.join("test.sqlite")).unwrap();
        let first = route_profile();
        let mut second = first.clone();
        second.id = "second".into();
        second.selected = false;
        store.upsert_model_profile(first).unwrap();
        store.upsert_model_profile(second).unwrap();
        let before_first = store.model_profile("profile").unwrap().unwrap();
        let before_second = store.model_profile("second").unwrap().unwrap();
        let snapshot = select_remote_profile(&store, "second", before_second.revision as u64).unwrap();
        let profiles = snapshot["profiles"].as_array().unwrap();
        assert_eq!(profiles.iter().filter(|p| p["selected"] == true).count(), 1);
        assert_eq!(profiles.iter().find(|p| p["selected"] == true).unwrap()["id"], "second");
        let after_first = store.model_profile("profile").unwrap().unwrap();
        let after_second = store.model_profile("second").unwrap().unwrap();
        assert!(after_first.revision > before_first.revision);
        assert!(after_second.revision > before_second.revision);
        assert_eq!(after_first.parameters_json, before_first.parameters_json);
        assert_eq!(after_second.source_json, before_second.source_json);
        assert!(select_remote_profile(&store, "profile", before_first.revision as u64).is_err());
        for source in [json!({"endpointEnabled":false,"capabilities":["chat"]}), json!({"modelEnabled":false,"capabilities":["chat"]}), json!({"capabilities":["retrieval"]})] {
            let mut unavailable = after_first.clone();
            unavailable.id = "unavailable".into();
            unavailable.selected = false;
            unavailable.source_json = source.to_string();
            store.upsert_model_profile(unavailable).unwrap();
            let current = store.model_profile("unavailable").unwrap().unwrap();
            assert!(select_remote_profile(&store, "unavailable", current.revision as u64).is_err());
            assert!(store.model_profile("second").unwrap().unwrap().selected);
        }
    }

    #[test]
    fn disabling_selected_remote_model_reselects_an_eligible_peer_atomically() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("models.sqlite")).unwrap();
        let first = route_profile();
        let mut second = first.clone();
        second.id = "second".into();
        second.selected = false;
        store.upsert_model_profile(first).unwrap();
        store.upsert_model_profile(second).unwrap();
        let before = store.model_profile("profile").unwrap().unwrap();
        configure_remote_profile_full(&store, "profile", before.revision as u64, json!({}), None,
            Some(&json!({"enabled":false}))).unwrap();
        let after_first = store.model_profile("profile").unwrap().unwrap();
        let after_second = store.model_profile("second").unwrap().unwrap();
        assert!(!after_first.selected);
        assert!(after_second.selected);
        assert!(after_second.revision > before.revision);
        assert!(resolve_remote_profile_settings(&after_first, &json!({}), &RuntimeConfig::default(), &[]).is_err());
        assert!(resolve_remote_profile_settings(&after_second, &json!({}), &RuntimeConfig::default(), &[]).is_ok());
    }

    #[test]
    fn model_default_thinking_is_inherited_but_explicit_session_selection_wins() {
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["reasoning"]["defaultPreference"] = json!("high");
        profile.source_json = source.to_string();
        for (request, expected) in [
            (json!({}), json!({"kind":"effort", "value":"high"})),
            (json!({"reasoning_selection":null}), Value::Null),
            (json!({"reasoning_selection":{"kind":"effort", "value":"low"}}), json!({"kind":"effort", "value":"low"})),
        ] {
            let resolved = resolve_remote_profile_settings(&profile, &request, &RuntimeConfig::default(), &[]).unwrap();
            assert_eq!(serde_json::to_value(resolved.runtime.reasoning_selection).unwrap(), expected);
        }
        source["reasoning"]["defaultPreference"] = json!("xhigh");
        profile.source_json = source.to_string();
        assert!(resolve_remote_profile_settings(&profile, &json!({}), &RuntimeConfig::default(), &[]).is_err());
        for request in [json!({"reasoning_selection":null}), json!({"reasoning_selection":{"kind":"effort","value":"low"}})] {
            assert!(resolve_remote_profile_settings(&profile, &request, &RuntimeConfig::default(), &[]).is_ok(),
                "A shadowed invalid default must not override a valid session choice");
        }
        assert!(resolve_remote_profile_settings(&profile, &json!({"reasoning_selection":{"kind":"effort","value":"xhigh"}}), &RuntimeConfig::default(), &[]).is_err());
    }

    #[test]
    fn default_thinking_tiers_use_the_effective_protocol_contract() {
        let effort = json!({"control":"effort","efforts":["low","medium","high"],"defaultPreference":"balanced"});
        assert!(validate_profile_reasoning(&effort, "chat_completions", 16384).is_ok());
        assert_eq!(serde_json::to_value(nola_application::resolve_model_reasoning(&effort, nola_domain::generation::ModelProtocol::ChatCompletions, 16384, true, None).unwrap()).unwrap(), json!({"kind":"effort","value":"medium"}));
        assert!(validate_profile_reasoning(&effort, "responses", 16384).is_err());
        let budget = json!({"control":"budget","budgets":[1024,4096,8192],"defaultPreference":"detailed"});
        assert!(validate_profile_reasoning(&budget, "anthropic", 16384).is_ok());
        assert_eq!(serde_json::to_value(nola_application::resolve_model_reasoning(&budget, nola_domain::generation::ModelProtocol::Anthropic, 16384, true, None).unwrap()).unwrap(), json!({"kind":"budget","tokens":8192}));
        assert!(validate_profile_reasoning(&budget, "anthropic", 4096).is_err());
        for invalid in [json!(42), json!("high")] {
            assert!(validate_profile_reasoning(&json!({"control":"automatic","defaultPreference":invalid}), "responses", 16384).is_err());
        }
    }

    #[test]
    fn profile_snapshot_reaches_real_responses_http_request() {
        responses_http_contract(false);
    }

    #[test]
    fn host_model_intent_reaches_real_responses_http_request() {
        responses_http_contract(true);
    }

    fn responses_http_contract(use_intent: bool) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", server.server_addr());
        let mut profile = route_profile();
        let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
        source["baseURL"] = json!(endpoint);
        source["reasoning"]["defaultPreference"] = json!("high");
        profile.source_json = source.to_string();
        let mut defaults = RuntimeConfig::default();
        defaults.endpoint = Some("http://invalid.global.example/v1".into());
        defaults.provider = Some("anthropic".into());
        defaults.model = Some("wrong-global-model".into());
        defaults.api_key = Some("wrong-global-key".into());
        let resolved = if use_intent {
            let mut requested = intent(&profile, json!({"max_output_tokens":32000}));
            requested.tier = nola_domain::reasoning_tiers::ReasoningTier::Concise;
            resolve_bound_model_intent_settings(&profile, &requested, &Value::Null,
                &defaults, &[(endpoint.as_str(), "fixture-key")]).unwrap()
        } else {
            resolve_remote_profile_settings(&profile, &json!({"max_output_tokens":32000}),
                &defaults, &[(endpoint.as_str(), "fixture-key")]).unwrap()
        };
        let receiver = std::thread::spawn(move || {
            let mut request = server.recv_timeout(std::time::Duration::from_secs(10)).unwrap().expect("model request");
            assert_eq!(request.url(), "/v1/responses");
            assert!(request.headers().iter().any(|header| header.field.equiv("Authorization") && header.value.as_str() == "Bearer fixture-key"));
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let body: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(body["model"], "fixture-model");
            assert_eq!(body["max_output_tokens"], 32000);
            assert_eq!(body["reasoning"]["effort"], if use_intent { "low" } else { "high" });
            assert_eq!(body["temperature"], 0.7);
            assert_eq!(body["top_p"], 0.9);
            let events = [json!({"type":"response.output_text.delta","delta":"ok"}),
                json!({"type":"response.completed","response":{"id":"fixture-response","object":"response","status":"completed","model":"fixture-model","output":[],"usage":{"input_tokens":1,"output_tokens":1}}})];
            let response = events.iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
            request.respond(tiny_http::Response::from_string(response).with_header(
                tiny_http::Header::from_bytes("Content-Type", "text/event-stream").unwrap())).unwrap();
        });
        let config = crate::llm::LlmConfig {
            siwc: None, native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider:resolved.runtime.provider, fallbacks:vec![], endpoint:resolved.endpoint,
            api_key:resolved.api_key, model:resolved.model, temperature:resolved.temperature, top_p:resolved.top_p,
            max_output_tokens:resolved.max_output_tokens, max_retries:0, parallel_tool_calls:None,
            thinking_enabled:true, reasoning_summary_supported:resolved.runtime.model_capabilities.reasoning_summary_supported, sampling_override:resolved.runtime.sampling_override,
            generation_options:resolved.runtime.generation_options, reasoning_selection:resolved.runtime.reasoning_selection,
            cancellation:Default::default(), openai_prompt_cache:false, openai_prompt_cache_key:None,
            openai_prompt_cache_key_prefix:None, openai_prompt_cache_retention:None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
        };
        let mut text = String::new();
        let result = crate::llm::stream_chat(&config, &[json!({"role":"user","content":"hello"})], &[], "auto", |event| {
            if let crate::llm::LlmEvent::TextDelta(delta) = event { text.push_str(&delta); }
        });
        receiver.join().unwrap();
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(text, "ok");
    }

    #[test]
    fn profile_inference_resolves_defaults_and_session_overrides_without_mutation() {
        let profile = route_profile();
        let mut defaults = RuntimeConfig::default();
        defaults.temperature = Some(1.5);
        defaults.max_output_tokens = Some(1024);
        defaults.provider = Some("anthropic".into());
        let resolved = resolve_remote_profile_settings(&profile, &json!({"temperature":0,"top_p":0.25,
            "max_output_tokens":32000,"reasoning_selection":{"kind":"effort","value":"high"}}), &defaults,
            &[("https://fixture.invalid/v1/","endpoint-key")]).unwrap();
        assert_eq!(resolved.model, "fixture-model");
        assert_eq!(resolved.api_key, "endpoint-key");
        assert_eq!(resolved.temperature, 0.0);
        assert_eq!(resolved.top_p, 0.25);
        assert_eq!(resolved.max_output_tokens, 32000);
        assert_eq!(resolved.max_context_tokens, 4000000);
        assert_eq!(resolved.runtime.provider.as_deref(), Some("openai-responses"));
        assert!(resolved.runtime.sampling_override);
        assert_eq!(defaults.temperature, Some(1.5));
        assert_eq!(defaults.provider.as_deref(), Some("anthropic"));
        let inherited = resolve_remote_profile_settings(&profile, &json!({}), &defaults, &[]).unwrap();
        assert_eq!(inherited.max_output_tokens, 64000);
        assert!(inherited.runtime.sampling_override);
        assert!(inherited.runtime.reasoning_selection.is_none());
    }

    #[test]
    fn model_requests_accept_only_current_wire_fields() {
        let profile = route_profile();
        for field in ["endpoint", "apiKey", "topP", "maxTokens", "contextWindowTokens"] {
            let mut request = json!({});
            request[field] = json!("obsolete");
            let error = resolve_remote_profile_settings(&profile, &request,
                &RuntimeConfig::default(), &[]).err().expect("obsolete wire field must fail");
            assert!(error.contains(field), "{error}");
        }
        assert!(crate::model_profile_resolution::validate_model_request_fields(&json!({
            "customEndpoint":"https://fixture.invalid/v1", "customApiKey":"key",
            "top_p":0.25, "max_output_tokens":8192, "max_context_tokens":64000
        })).is_ok());
    }

    #[test]
    fn profile_credentials_never_cross_endpoint_boundaries() {
        let profile = route_profile();
        let defaults = RuntimeConfig::default();
        let resolved = resolve_remote_profile_settings(&profile, &json!({}), &defaults,
            &[("https://other.invalid/v1","unrelated-key"),("https://fixture.invalid/other","wrong-path-key")]).unwrap();
        assert_eq!(resolved.api_key, "");
        assert_eq!(resolved.runtime.api_key.as_deref(), Some(""));
        assert!(resolve_remote_profile_settings(&profile, &json!({"customEndpoint":"https://other.invalid/v1"}), &defaults, &[]).is_err());
        assert!(resolve_remote_profile_settings(&profile, &json!({"model":"another-model"}), &defaults, &[]).is_err());
        let explicit = resolve_remote_profile_settings(&profile, &json!({"customApiKey":"explicit-key"}), &defaults, &[]).unwrap();
        assert_eq!(explicit.api_key, "explicit-key");
    }

    #[test]
    fn profile_selection_rejects_disabled_deleted_and_undeclared_efforts() {
        let mut profile = route_profile();
        let defaults = RuntimeConfig::default();
        assert!(resolve_remote_profile_settings(&profile, &json!({"reasoning_selection":{"kind":"effort","value":"xhigh"}}), &defaults, &[]).is_err());
        profile.enabled = false;
        assert!(resolve_remote_profile_settings(&profile, &json!({}), &defaults, &[]).is_err());
        profile.enabled = true;
        profile.deleted_at_ms = Some(2);
        assert!(resolve_remote_profile_settings(&profile, &json!({}), &defaults, &[]).is_err());
    }

    #[test]
    fn sparse_generation_overrides_preserve_other_defaults_and_allow_seed_clear() {
        let mut profile = route_profile();
        profile.source_json = json!({"baseURL":"https://fixture.invalid/v1","capabilities":["chat"],"wireProtocol":"chat_completions",
            "executionCapabilities": nola_application::ModelExecutionCapabilities::default()}).to_string();
        profile.parameters_json = json!({"presencePenalty":0.5,"frequencyPenalty":0.3,"seed":42}).to_string();
        let resolved = resolve_remote_profile_settings(&profile, &json!({"generation_options":{"seed":null,"presence_penalty":0}}), &RuntimeConfig::default(), &[]).unwrap();
        assert_eq!(resolved.runtime.generation_options.seed, None);
        assert_eq!(resolved.runtime.generation_options.presence_penalty, Some(0.0));
        assert_eq!(resolved.runtime.generation_options.frequency_penalty, Some(0.3));
    }

    #[test]
    fn model_settings_preserve_large_values_and_reject_invalid_aliases() {
        let input = json!({"maxOutputTokens":64000,"maxContextTokens":4000000,"temperature":0});
        validate_model_setting_values(&input).unwrap();
        let mut settings = AgentSettings::default();
        merge_agent_settings(&mut settings, input);
        assert_eq!(settings.max_output_tokens, 64000);
        assert_eq!(settings.max_context_tokens, 4000000);
        assert_eq!(settings.temperature, 0.0);
        assert!(validate_model_setting_values(&json!({"maxOutputTokens":"64000"})).is_err());
        assert!(validate_model_setting_values(&json!({"max_output_tokens":4096,"maxOutputTokens":64000})).is_err());
        assert!(validate_model_setting_values(&json!({"maxContextTokens":4000001})).is_err());
    }

    #[test]
    fn remote_parameter_edit_is_canonical_and_rejects_stale_revision() {
        let root = std::env::temp_dir().join(format!("nola-web-model-profile-{}-{}", std::process::id(), chrono::Utc::now().timestamp_nanos_opt().unwrap()));
        fs::create_dir_all(&root).unwrap();
        let store = NolaStore::open(&root.join("test.sqlite")).unwrap();
        let original = nola_application::ModelProfile {
            id:"fixture".into(), display_name:"Fixture".into(), model_name:"model".into(),
            provider:"openai-compatible".into(), runtime:"remote".into(),
            source_json:json!({"schemaVersion":1, "endpointID":"endpoint", "wireProtocol":"responses",
                "executionCapabilities":{"toolCalls":false,"imageInput":false,"reasoning":true,"samplingParameters":true}}).to_string(),
            parameters_json:json!({"temperature":0.7,"maxOutputTokens":16384,"showThinking":true}).to_string(),
            selected:true, enabled:true, revision:0, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        };
        store.upsert_model_profile(original.clone()).unwrap();
        let initial = store.model_profile("fixture").unwrap().unwrap();
        let result = configure_remote_profile(&store, "fixture", initial.revision as u64, json!({"temperature":0})).unwrap();
        assert_eq!(result["parameters"]["temperature"], 0);
        let persisted = store.model_profile("fixture").unwrap().unwrap();
        assert!(persisted.selected);
        assert_eq!(persisted.source_json, original.source_json);
        assert_eq!(serde_json::from_str::<Value>(&persisted.parameters_json).unwrap()["showThinking"], true);
        assert!(configure_remote_profile(&store, "fixture", initial.revision as u64, json!({"temperature":1.5})).is_err());
        assert_eq!(store.model_profile("fixture").unwrap().unwrap().parameters_json, persisted.parameters_json);
        let configured = configure_remote_profile_settings(&store, "fixture", persisted.revision as u64,
            json!({"maxOutputTokens":64000}), Some(json!({"control":"effort", "efforts":["high", "xhigh"], "defaultPreference":"high"}))).unwrap();
        assert_eq!(configured["reasoning"]["defaultPreference"], "high");
        assert_eq!(configured["reasoning"]["responses"], true);
        assert_eq!(configured["reasoning"]["outputLimit"], 64000);
        assert_eq!(configured["reasoning"]["budgets"], json!([1024, 4096, 8192]));
        assert_eq!(configured["selected"], true);
        assert_eq!(configured["parameters"]["temperature"], 0);
        assert!(configure_remote_profile_settings(&store, "fixture", persisted.revision as u64,
            json!({}), Some(json!({"control":"automatic"}))).is_err());
        assert!(configure_remote_profile_settings(&store, "fixture", configured["revision"].as_u64().unwrap(),
            json!({"temperature":1.8}), Some(json!({"control":"budget", "budgets":[1024,4096,8192]}))).is_err());
        assert_eq!(remote_profile_projection(&store.model_profile("fixture").unwrap().unwrap()).unwrap(), configured,
            "Invalid reasoning must not partially save numeric parameters");
        let revision = configured["revision"].as_u64().unwrap();
        for invalid in [json!({"priority":101}), json!({"enabled":"false"}), json!({"displayName":"  "}),
            json!({"capabilities":["unknown"]}), json!({"capabilities":["chat","chat"]}), json!({"apiKey":"secret"})] {
            assert!(configure_remote_profile_full(&store, "fixture", revision, json!({"temperature":1.1}), None, Some(&invalid)).is_err());
            assert_eq!(remote_profile_projection(&store.model_profile("fixture").unwrap().unwrap()).unwrap(), configured);
        }
        let metadata = json!({"displayName":" Renamed ","enabled":false,"priority":0,"capabilities":["chat","retrieval"]});
        let edited = configure_remote_profile_full(&store, "fixture", revision, json!({"temperature":0.25}), None, Some(&metadata)).unwrap();
        assert_eq!(edited["displayName"], "Renamed");
        assert_eq!(edited["enabled"], false);
        assert_eq!(edited["priority"], 0);
        assert_eq!(edited["parameters"]["temperature"], 0.25);
        assert_eq!(edited["endpoint"], configured["endpoint"]);
        assert_eq!(edited["reasoning"], configured["reasoning"]);
        assert_eq!(edited["selected"], true);
        assert!(configure_remote_profile_full(&store, "fixture", revision, json!({}), None, Some(&json!({"enabled":true}))).is_err());
        assert_eq!(remote_profile_projection(&store.model_profile("fixture").unwrap().unwrap()).unwrap(), edited);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_parameters_accept_zero_and_explicit_seed_clear() {
        assert!(validate_remote_parameter_patch(&json!({"temperature":0,"seed":null,"maxOutputTokens":64000})).is_ok());
        for value in [json!({"temperature":3}), json!({"maxOutputTokens":0}), json!({"apiKey":"secret"}), json!({"maxContextLength":"128000"})] {
            assert!(validate_remote_parameter_patch(&value).is_err());
        }
    }

    #[test]
    fn reasoning_contract_rejects_protocol_and_effective_output_mismatches() {
        let budget = json!({"control":"budget", "budgets":[1024,4096,8192]});
        assert!(validate_profile_reasoning(&budget, "anthropic", 16384).is_ok());
        assert!(validate_profile_reasoning(&budget, "anthropic", 8192).is_err());
        assert!(validate_profile_reasoning(&budget, "responses", 16384).is_err());
        let effort = json!({"control":"effort", "efforts":["high"]});
        assert!(validate_profile_reasoning(&effort, "responses", 16384).is_ok());
        assert!(validate_profile_reasoning(&effort, "anthropic", 16384).is_err());
        assert!(validate_profile_reasoning(&effort, "chat_completions", 16384).is_err());
        assert!(validate_profile_reasoning(&json!({"control":"automatic","efforts":"high"}), "responses", 16384).is_err());
        assert!(validate_profile_reasoning(&Value::Null, "responses", 16384).is_ok());
    }

    #[test]
    fn remote_projection_does_not_expose_arbitrary_source_fields() {
        let profile = nola_application::ModelProfile {
            id:"fixture".into(), display_name:"Fixture".into(), model_name:"model".into(),
            provider:"openai-compatible".into(), runtime:"remote".into(),
            source_json:json!({"endpointID":"endpoint", "wireProtocol":"responses", "apiKey":"never-publish"}).to_string(),
            parameters_json:"{\"temperature\":0}".into(), selected:true, enabled:true,
            revision:2, created_at_ms:0, updated_at_ms:0, deleted_at_ms:None,
        };
        let projection = remote_profile_projection(&profile).unwrap();
        assert!(!projection.to_string().contains("never-publish"));
        assert_eq!(projection["endpoint"]["wireProtocol"], "responses");
        assert_eq!(projection["parameters"]["temperature"], 0);
    }
}
