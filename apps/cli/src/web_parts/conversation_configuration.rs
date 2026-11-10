// The review describes this authenticated Host's configured workspace. It is
// unavailable for remote-owned or remote-bound conversations, including when
// a local path happens to match one recorded by the remote Host.
fn workspace_review_owner(id: &str) -> Result<i64, String> {
    let path = crate::platform_paths::application_database_path()?;
    let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
    let owner: nola_domain::ConversationSourceOwner = app.execute(
        nola_application::ApplicationCommand::LoadConversationSourceOwner { conversation_id:id.into() })?;
    if owner != nola_domain::ConversationSourceOwner::Local { return Err("workspace_review_requires_local_source".into()); }
    #[derive(serde::Deserialize)]
    struct ReviewMetadata { id:String, revision:i64 }
    let metadata: Option<ReviewMetadata> = app.execute(
        nola_application::ApplicationCommand::LoadConversationMetadata { conversation_id:id.into() })?;
    let metadata = metadata.ok_or("workspace_review_conversation_not_found")?;
    if metadata.id != id || metadata.revision < 1 { return Err("workspace_review_metadata_invalid".into()); }
    let Some((revision, settings)) = conversation_configuration_state(id)? else {
        return Err("workspace_review_conversation_not_found".into());
    };
    if !settings["executionHostBinding"].is_null()
        || settings["executionHostRawValue"].as_str().is_some_and(|host|host != "local") {
        return Err("workspace_review_requires_local_source".into());
    }
    Ok(revision)
}

fn conversation_workspace_review_response(id: &str, config: &Config) -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Value,String> {
        let owner = workspace_review_owner(id)?;
        let review = crate::tools::read_host_workspace_review(&config.project_root)?;
        if workspace_review_owner(id)? != owner { return Err("workspace_review_owner_changed".into()); }
        Ok(json!({"source_host_id":host_identity(),"conversation_id":id,"workspace_root":config.project_root,"review":review}))
    })();
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(StatusCode(409), json!({"error":error})),
    }
}

// One Application read snapshot owns all Today facts. The Web adapter never
// combines separately refreshed permission, child-task or schedule projections.
fn workspace_today_response(url: &str) -> Response<Box<dyn Read + Send>> {
    let offset = match query_param(url, "offset") {
        None => 0,
        Some(value) => match value.parse::<usize>() {
            Ok(value) if value <= 1_000_000 => value,
            _ => return json_response(StatusCode(400), json!({"error":"invalid Today offset"})),
        },
    };
    let result = (|| -> Result<nola_application::TodayFactsSnapshot, String> {
        let path = crate::platform_paths::application_database_path()?;
        let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
        app.execute(nola_application::ApplicationCommand::LoadTodayFacts {
            input: nola_application::TodayFactsQuery {
                conversation_offset: offset, conversation_limit: 50, queued_limit: 10,
            },
        })
    })();
    match result {
        Ok(snapshot) => json_response(StatusCode(200), json!({"source_host_id":host_identity(), "snapshot":snapshot})),
        Err(error) => json_response(StatusCode(500), json!({"error":error})),
    }
}

// Canonical settings_json is the sole source of persisted session configuration.
// Read through Application: clients must not sum percentages or count replayed
// model events. The durable request ledger owns conversation-wide cache totals.
fn conversation_context_usage_response(id: &str) -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Option<Value>, String> {
        let path = crate::platform_paths::application_database_path()?;
        let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
        if app.load_conversation(id)?.is_none() {
            return Ok(None);
        }
        let usage: nola_domain::ContextUsageView = app.execute(
            nola_application::ApplicationCommand::LoadConversationContextUsageV1 {
                conversation_id: id.to_owned(),
            },
        )?;
        Ok(Some(json!({ "conversation_id": id, "usage": usage })))
    })();
    match result {
        Ok(Some(value)) => json_response(StatusCode(200), value),
        Ok(None) => not_found("Conversation not found"),
        Err(error) => json_response(StatusCode(500), json!({"error": error})),
    }
}

fn conversation_configuration_state(id: &str) -> Result<Option<(i64, Value)>, String> {
    let path = crate::platform_paths::application_database_path()?;
    let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Metadata { id: String, revision: i64, configuration_revision: i64, settings_json: Option<String> }
    // The canonical query pins one snapshot and does not hydrate message bodies.
    let Some(metadata) = app.execute::<Option<Metadata>>(
        nola_application::ApplicationCommand::LoadConversationMetadata { conversation_id: id.into() }
    )? else { return Ok(None) };
    if metadata.id != id || metadata.revision < 1 {
        return Err("invalid conversation metadata identity or revision".into());
    }
    let value: Value = serde_json::from_str(metadata.settings_json.as_deref().unwrap_or("{}"))
        .map_err(|_| "invalid conversation settings")?;
    if !value.is_object() { return Err("conversation settings must be an object".into()); }
    Ok(Some((metadata.configuration_revision, value)))
}

fn configuration_projection(id: &str, revision: i64, settings: &Value) -> Value {
    json!({"conversationId":id,"configurationRevision":revision,
        "modelSelectionID":settings.get("modelSelectionID").cloned().unwrap_or(Value::Null),
        "executionHostRawValue":settings.get("executionHostRawValue").cloned().unwrap_or(Value::Null),
        "executionHostBinding":settings.get("executionHostBinding").cloned().unwrap_or(Value::Null),
        "clientHostParameters":settings.get("clientHostParameters").cloned().unwrap_or(Value::Null),
        "clientModelPresentation":settings.get("clientModelPresentation").cloned().unwrap_or(json!({})),
        "knowledge":settings.get("knowledge").cloned().unwrap_or(Value::Null),
        "modelOverrides":settings.get("modelOverrides").cloned().unwrap_or(json!({})),
        "memoryEnabledOverride":settings.get("memoryEnabledOverride").cloned().unwrap_or(Value::Null),
        "retrievalEnabledOverride":settings.get("retrievalEnabledOverride").cloned().unwrap_or(Value::Null),
        "isPinned":settings.get("isPinned").cloned().unwrap_or(json!(false)),
        "language":settings.get("language").cloned().unwrap_or(Value::Null),
        "selectedPromptTemplateID":settings.get("selectedPromptTemplateID").cloned().unwrap_or(Value::Null),
        "promptResponseStyle":settings.get("promptResponseStyle").cloned().unwrap_or(Value::Null),
        "executionBackend":settings.get("executionBackend").cloned().unwrap_or(Value::Null),
        "externalExecutionSessions":settings.get("externalExecutionSessions").cloned().unwrap_or(Value::Null),
        "dataSourceOverrides":settings.get("dataSourceOverrides").cloned().unwrap_or(Value::Null),
        "dataSourceIds":settings.get("dataSourceIds").cloned().unwrap_or(Value::Null),
        "persistenceEnabled":settings.get("persistenceEnabled").cloned().unwrap_or(Value::Null),
        "promptTemplateSelectionIsExplicit":settings.get("promptTemplateSelectionIsExplicit").cloned().unwrap_or(Value::Null),
        "contextLimitOverride":settings.get("contextLimitOverride").cloned().unwrap_or(Value::Null),
        "externalExecutionModelIDs":settings.get("externalExecutionModelIDs").cloned().unwrap_or(Value::Null),
        "externalExecutionModelProviderIDs":settings.get("externalExecutionModelProviderIDs").cloned().unwrap_or(Value::Null),
        "executionPolicy":settings.get("executionPolicy").cloned().unwrap_or(Value::Null),
        "permissionProfile":settings.get("permissionModeRawValue").cloned().unwrap_or(Value::Null)})
}

fn conversation_configuration_response(id: &str, minimum_policy_revision: Option<i64>) -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Option<Value>, String> {
        let path = crate::platform_paths::application_database_path()?;
        let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
        let snapshot: Value = app.execute(nola_application::ApplicationCommand::ReadPermissionConfiguration {
            conversation_id:id.into(), source_host_id:None, client_id:None,
            unresolved_run_ids:Vec::new(), source_unconfirmed:false, minimum_policy_revision,
        })?;
        if snapshot.is_null() { return Ok(None); }
        let revision = snapshot["configuration_revision"].as_i64().ok_or("missing configuration revision")?;
        let mut value = configuration_projection(id, revision, &snapshot["settings"]);
        value["activeRunID"] = snapshot["active_run_id"].clone();
        value["activePermissionPolicy"] = snapshot["active_policy"].clone();
        value["permissionState"] = snapshot["state"].clone();
        value["permissionOwner"] = snapshot["owner"].clone();
        Ok(Some(value))
    })();
    match result {
        Ok(Some(value)) => json_response(StatusCode(200), value),
        Ok(None) => not_found("Conversation not found"),
        Err(error) => json_response(StatusCode(500), json!({"error":error})),
    }
}

fn patch_conversation_configuration_response(
    request: &mut Request,
    id: &str,
) -> Response<Box<dyn Read + Send>> {
    let mut patch = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(object) = patch.as_object_mut() else {
        return json_response(
            StatusCode(422),
            json!({"code":"invalid_conversation_configuration"}),
        );
    };
    let Some(expected_revision) = object
        .remove("expectedRevision")
        .and_then(|value| value.as_i64())
        .filter(|value| *value > 0)
    else {
        return json_response(
            StatusCode(400),
            json!({"code":"configuration_revision_required"}),
        );
    };
    let raw = patch.to_string();
    if let Err(error) = nola_application::conversation_configuration::validate_patch(&raw) {
        return json_response(
            StatusCode(422),
            json!({"code":"invalid_conversation_configuration","error":error}),
        );
    }
    let (revision, settings) = match conversation_configuration_state(id) {
        Ok(Some(value)) => value,
        Ok(None) => return json_response(StatusCode(404),json!({"status":"not_found","conversation_id":id})),
        Err(error) => return json_response(StatusCode(500), json!({"error":error})),
    };
    if revision != expected_revision {
        return json_response(
            StatusCode(409),
            json!({"status":"conflict","conversation_id":id,"expected_revision":expected_revision,"actual_revision":revision}),
        );
    }
    // Validate merged declarations against real profile contracts before mutation;
    // runtime repeats validation because a profile may change after this patch.
    if let Some(models) = patch["modelOverrides"].as_object() {
        for (key, change) in models {
            if change.is_null() {
                continue;
            }
            let mut merged = settings["modelOverrides"][key]
                .as_object()
                .cloned()
                .unwrap_or_default();
            for (field, value) in change.as_object().unwrap() {
                if value.is_null() {
                    merged.remove(field);
                } else {
                    merged.insert(field.clone(), value.clone());
                }
            }
            if let Err(error) = validate_session_model_override(key, &Value::Object(merged)) {
                return json_response(
                    StatusCode(422),
                    json!({"code":"invalid_conversation_configuration","error":error}),
                );
            }
        }
    }
    use nola_domain::conversation_configuration::ConfigurationCommitResult;
    let result = (|| -> Result<ConfigurationCommitResult, String> {
        let path = crate::platform_paths::application_database_path()?;
        let mut app = crate::application_port::CliApplicationPort::persistent(&path)?;
        if let Some(profile) = patch.get("permissionProfile") {
            let mut additional = patch.as_object().cloned().ok_or("invalid configuration patch")?;
            additional.remove("permissionProfile");
            let additional_configuration = if additional.is_empty() { None } else {
                Some(Value::Object(additional).to_string().parse()?)
            };
            let owner: Value = app.execute(nola_application::ApplicationCommand::ReadPermissionConfiguration {
                conversation_id:id.into(), source_host_id:None, client_id:None,
                unresolved_run_ids:Vec::new(), source_unconfirmed:false, minimum_policy_revision:None,
            })?;
            return app.execute(nola_application::ApplicationCommand::SetConversationPermission {
                conversation_id:id.into(), expected_revision,
                expected_run_id:owner["active_run_id"].as_str().map(str::to_owned),
                expected_source_host_id:None, profile:serde_json::from_value(profile.clone()).map_err(|e|e.to_string())?, additional_configuration,
                updated_at_ms:Utc::now().timestamp_millis(),
            });
        }
        app.execute(nola_application::ApplicationCommand::PatchConversationConfiguration {
            input: nola_application::conversation_configuration::PatchConversationConfigurationInput {
                conversation_id:id.into(),expected_revision,patch_json:raw.parse()?,updated_at_ms:Utc::now().timestamp_millis(),
            },
        })
    })();
    match result {
        Ok(value @ ConfigurationCommitResult::Committed { .. }) => json_response(StatusCode(200), json!(value)),
        Ok(value @ ConfigurationCommitResult::Conflict { .. }) => json_response(StatusCode(409), json!(value)),
        Ok(value @ ConfigurationCommitResult::NotFound { .. }) => json_response(StatusCode(404), json!(value)),
        Err(error) => json_response(StatusCode(500), json!({"error":error})),
    }
}

#[cfg(test)]
mod session_override_translation_tests {
    use super::*;

    #[test]
    fn external_submission_does_not_require_an_unrelated_http_model() {
        let _lock = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
        impl Drop for Environment {
            fn drop(&mut self) {
                for (key, value) in &self.0 {
                    match value { Some(value) => std::env::set_var(key, value), None => std::env::remove_var(key) }
                }
            }
        }
        let _environment = Environment(["NOLA_HOME", "NOLA_DB_PATH"].into_iter()
            .map(|key| (key, std::env::var_os(key))).collect());
        std::env::set_var("NOLA_HOME", root.path());
        std::env::set_var("NOLA_DB_PATH", root.path().join("external.sqlite"));
        let conversation = WebConversation::new("external-no-http".into(), "Fixture".into(), now_string(), now_string());
        NolaStore::open_default().unwrap().create_web_conversation(&json!(conversation)).unwrap();
        let request = json!({"conversationId":"external-no-http","execution_backend":"codex","permission_profile":"cautious"});
        let applied = apply_conversation_request_configuration(request.clone()).unwrap();
        assert_eq!(applied["permission_profile"], "cautious");
        assert!(applied.get("modelProfileId").is_none());
        let mut nola = request; nola["execution_backend"] = json!("nola");
        assert!(apply_conversation_request_configuration(nola).unwrap_err().contains("no saved model selection"));
    }

    #[test]
    fn configuration_get_preserves_mobile_bindings_without_exposing_display_snapshots() {
        let binding = json!({"baseURL":"https://host.example/v1"});
        let parameters = json!({"maxOutputTokens":2048, "temperature":0.3});
        let presentation = json!({"execution-host":{"showThinking":false,"streamOutput":true}});
        let projected = configuration_projection("mobile", 9, &json!({
            "executionHostRawValue":"remote",
            "executionHostBinding":binding,
            "clientHostParameters":parameters,
            "clientModelPresentation":presentation,
            "selectedModelInfo":{"apiKey":"must-not-project"}
        }));
        assert_eq!(projected["executionHostBinding"], binding);
        assert_eq!(projected["executionHostRawValue"], "remote");
        assert_eq!(projected["clientHostParameters"], parameters);
        assert_eq!(projected["clientModelPresentation"], presentation);
        assert!(projected["modelSelectionID"].is_null());
        assert!(projected.get("selectedModelInfo").is_none());
        let empty = configuration_projection("empty", 1, &json!({}));
        assert!(empty["executionHostBinding"].is_null());
        assert!(empty["executionHostRawValue"].is_null());
        assert!(empty["clientHostParameters"].is_null());
        assert_eq!(empty["clientModelPresentation"], json!({}));
    }

    #[test]
    fn shared_knowledge_projection_has_exact_owner_revision_and_native_document() {
        let document = json!({"keyPointsMarkdown":"  事实\n", "wikiMarkdown":"# Wiki\n", "updatedAt":1.5});
        let settings = json!({"knowledge":document, "conversation":{"knowledgePoints":"separate old data"}});
        let projected = configuration_projection("one", 7, &settings);
        assert_eq!(projected["conversationId"], "one");
        assert_eq!(projected["configurationRevision"], 7);
        assert_eq!(projected["knowledge"], document);
        assert!(projected.get("knowledgePoints").is_none());
        assert!(configuration_projection("empty", 1, &json!({}))["knowledge"].is_null());
    }

    #[test]
    fn seed_override_preserves_inheritance_clear_zero_and_full_width_values() {
        let inherited = session_override_request(&json!({}), &Value::Null, "chat_completions").unwrap();
        assert!(inherited.get("generation_options").is_none());
        let cleared = session_override_request(&json!({"seed":{"kind":"clear"}}), &Value::Null, "chat_completions").unwrap();
        assert_eq!(cleared["generation_options"].as_object().unwrap().get("seed"), Some(&Value::Null));
        for seed in [i64::MIN, -9_007_199_254_740_993, -1, 0, 9_007_199_254_740_993, i64::MAX] {
            let request = session_override_request(&json!({"seed":{"kind":"value","value":seed.to_string()}}), &Value::Null, "chat_completions").unwrap();
            assert_eq!(request["generation_options"]["seed"].as_i64(), Some(seed));
        }
        assert_eq!(session_override_request(&json!({"seed":null}), &Value::Null, "chat_completions").unwrap(), inherited);
        for invalid in [json!(0), json!({"kind":"value","value":"-0"}), json!({"kind":"clear","value":"0"})] {
            assert!(session_override_request(&json!({"seed":invalid}), &Value::Null, "chat_completions").is_err());
        }
    }

    #[test]
    fn translation_does_not_silently_erase_invalid_top_k() {
        let request = session_override_request(&json!({"topK":0,"seed":{"kind":"value","value":"0"}}), &Value::Null, "chat_completions").unwrap();
        assert_eq!(request["generation_options"]["top_k"], json!(0));
        assert_eq!(request["generation_options"]["seed"], json!(0));
        assert!(crate::runtime_config::RuntimeConfig::default().with_model_request(&request).is_err());
    }

    #[test]
    fn stored_tier_uses_shared_translation_without_guessing_output_limit() {
        let declaration = json!({"control":"budget","budgets":[1024,4096,32768]});
        let request = session_override_request(&json!({"reasoningPreference":"detailed","maxOutputTokens":64000}),
            &declaration, "anthropic").unwrap();
        assert_eq!(request["reasoning_selection"], json!({"kind":"budget","tokens":32768}));
        assert!(crate::runtime_config::RuntimeConfig::default().with_model_request(&request).is_ok());
    }
}

use crate::model_profile_resolution::{session_override_request, profile_with_session_contract};

fn validate_session_model_override(key: &str, overrides: &Value) -> Result<(), String> {
    let store = NolaStore::open_default()?;
    if let Some(id) = key.strip_prefix("configured:") {
        let profile = store
            .model_profile(id)?
            .ok_or("session model profile is unavailable")?;
        let profile = profile_with_session_contract(&profile, overrides)?;
        let source: Value =
            serde_json::from_str(&profile.source_json).map_err(|_| "invalid model source")?;
        let protocol = source["wireProtocol"]
            .as_str()
            .unwrap_or("chat_completions");
        let request = session_override_request(overrides, &source["reasoning"], protocol)?;
        resolve_remote_profile_settings(
            &profile,
            &request,
            &crate::runtime_config::RuntimeConfig::default(),
            &[],
        )?;
    } else if let Some(id) = key.strip_prefix("local:") {
        if !local_models::contains_model(id) {
            return Err("local session model is unavailable".into());
        }
        let protocol = overrides["wireProtocol"]
            .as_str()
            .unwrap_or("chat_completions");
        let request = session_override_request(overrides, &overrides["reasoning"], protocol)?;
        crate::runtime_config::RuntimeConfig::default().with_model_request(&request)?;
    } else {
        return Err("unsupported session model identity".into());
    }
    Ok(())
}

/// One frozen snapshot per submission. Explicit per-turn controls override the
/// persisted session; missing controls inherit session before provider defaults.
fn apply_conversation_request_configuration(mut body: Value) -> Result<Value, String> {
    let intent = decode_run_model_intent(&body)?;
    let object = body.as_object_mut().ok_or("request must be an object")?;
    object.remove("__nola_session_model_contract");
    let id = object
        .get("conversationId")
        .or_else(|| object.get("session_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let Some(id) = id else {
        return Ok(body);
    };
    let Some((_, settings)) = conversation_configuration_state(&id)? else {
        return Ok(body);
    };
    if !object.contains_key("permission_profile") {
        if let Some(value) = settings.get("permissionModeRawValue") {
            if !value
                .as_str()
                .is_some_and(|value| ["cautious", "allow_all"].contains(&value))
            {
                return Err("invalid stored session permission mode".into());
            }
            object.insert("permission_profile".into(), value.clone());
        }
    }
    // An external executor owns its model/account. Do not require or bind an
    // unrelated Nola HTTP profile, while retaining Host and permission fences.
    if object.get("execution_backend").and_then(Value::as_str).is_some_and(|backend| backend != "nola") {
        if intent.is_some() { return Err("model_intent is only supported by the Nola execution backend".into()); }
        if !settings["executionHostBinding"].is_null() {
            return Err("Conversation is bound to another execution Host".into());
        }
        return Ok(body);
    }
    if intent.is_some() {
        // The intent resolver reads the matching Host session preferences with
        // the bound profile. Do not translate them through the legacy route.
        return Ok(body);
    }
    if !object.contains_key("modelProfileId") && !object.contains_key("model") {
        if !settings["executionHostBinding"].is_null() {
            return Err("Conversation is bound to another execution Host; explicitly select a model to run it here".into());
        }
        let native = settings["modelSelectionID"].as_str();
        let web = &settings["conversation"];
        if let Some(id) = native.and_then(|value| value.strip_prefix("configured:")) {
            object.insert("modelProfileId".into(), json!(id));
        } else if let Some(id) = native.and_then(|value| value.strip_prefix("local:")) {
            object.insert("model".into(), json!(id));
        } else if let Some(id) = web["selectedModelId"].as_str().filter(|id| !id.is_empty()) {
            if web["selectedModelSource"] == "local" {
                object.insert("model".into(), json!(id));
            } else {
                object.insert("modelProfileId".into(), json!(id));
            }
        } else {
            return Err("Conversation has no saved model selection; choose a model for this conversation".into());
        }
    }
    let key = object
        .get("modelProfileId")
        .and_then(Value::as_str)
        .map(|id| format!("configured:{id}"))
        .or_else(|| {
            object
                .get("model")
                .and_then(Value::as_str)
                .filter(|id| local_models::contains_model(id))
                .map(|id| format!("local:{id}"))
        });
    let Some(key) = key else {
        return Ok(body);
    };
    let Some(overrides) = settings
        .get("modelOverrides")
        .and_then(|models| models.get(&key))
    else {
        return Ok(body);
    };
    validate_session_model_override(&key, overrides)?;
    let (reasoning, protocol) = if let Some(id) = key.strip_prefix("configured:") {
        let profile = NolaStore::open_default()?
            .model_profile(id)?
            .ok_or("session model profile is unavailable")?;
        let profile = profile_with_session_contract(&profile, overrides)?;
        let source: Value =
            serde_json::from_str(&profile.source_json).map_err(|_| "invalid model source")?;
        (
            source["reasoning"].clone(),
            source["wireProtocol"]
                .as_str()
                .unwrap_or("chat_completions")
                .to_string(),
        )
    } else {
        (
            overrides["reasoning"].clone(),
            overrides["wireProtocol"]
                .as_str()
                .unwrap_or("chat_completions")
                .to_string(),
        )
    };
    let mut inherited = session_override_request(overrides, &reasoning, &protocol)?;
    // Normalize aliases so an explicit request cannot accidentally lose to a
    // persisted canonical spelling while preserving explicit zero and null.
    for (alias, canonical) in [
        ("topP", "top_p"),
        ("maxTokens", "max_output_tokens"),
        ("contextWindowTokens", "max_context_tokens"),
    ] {
        if let Some(value) = object.get(alias).cloned() {
            object.entry(canonical.to_string()).or_insert(value);
        }
    }
    if let (Some(base), Some(explicit)) = (
        inherited
            .get_mut("generation_options")
            .and_then(Value::as_object_mut),
        object.get("generation_options").and_then(Value::as_object),
    ) {
        base.extend(
            explicit
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        object.remove("generation_options");
    }
    inherited.as_object_mut().unwrap().extend(object.clone());
    inherited["__nola_session_model_contract"] = overrides.clone();
    Ok(inherited)
}
