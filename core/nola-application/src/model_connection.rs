// SPDX-License-Identifier: Apache-2.0
//! Connection commands project explicitly enabled routes into existing profiles.
use nola_domain::{generation::ModelProtocol, model_connection::*};
use nola_store_port::model::{ModelRepository, StoredModelProfile};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The canonical initial profile document for every remote-model creation path.
/// Absence of local sampling options is intent, not a request for local defaults.
pub fn remote_profile_parameters(context_window: Option<u64>, output_limit: Option<u64>) -> Value {
    json!({"maxContextLength":context_window.unwrap_or(128000),
        "maxOutputTokens":output_limit.unwrap_or(16384),"topK":null,"topP":0.9,"temperature":0.7,
        "presencePenalty":0.0,"frequencyPenalty":0.0,"repetitionPenalty":null,"seed":null,
        "stopSequences":"","showThinking":true,"streamOutput":true,"responseStyle":"casual","customSystemPrompt":""})
}

fn profiles(store: &impl ModelRepository, id: &str) -> Result<Vec<StoredModelProfile>, String> {
    Ok(store
        .list_model_profiles()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|p| {
            p.deleted_at_ms.is_none()
                && serde_json::from_str::<Value>(&p.source_json)
                    .ok()
                    .is_some_and(|s| s["connectionID"].as_str() == Some(id))
        })
        .collect())
}

fn revisions(profiles: &[StoredModelProfile]) -> BTreeMap<String, i64> {
    profiles
        .iter()
        .map(|p| (p.id.clone(), p.revision))
        .collect()
}

fn load(
    store: &impl ModelRepository,
    id: &str,
    expected: i64,
) -> Result<ModelConnectionSnapshot, String> {
    let current = store
        .model_connection(id)
        .map_err(|e| e.to_string())?
        .ok_or("model_connection_not_found")?;
    if current.revision != expected {
        return Err("model_connection_revision_conflict".into());
    }
    Ok(current)
}

fn commit(
    store: &mut impl ModelRepository,
    document: &ModelConnectionDocument,
    expected: Option<i64>,
    profiles: &[StoredModelProfile],
    revisions: &BTreeMap<String, i64>,
    at: i64,
) -> Result<Value, String> {
    document.validate()?;
    serde_json::to_value(
        store
            .commit_model_connection(document, expected, profiles, revisions, at)
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub(crate) fn save_connection(
    store: &mut impl ModelRepository,
    input: ModelConnection,
    expected: Option<i64>,
    at: i64,
) -> Result<Value, String> {
    input.validate()?;
    if input.source == ModelSourceKind::NolaChatgpt {
        let account = store.model_account(input.account_id.as_deref().ok_or("SIWC account missing")?)
            .map_err(|e| e.to_string())?.ok_or("SIWC account unavailable")?;
        if account.account.source != ModelSourceKind::NolaChatgpt {
            return Err("SIWC connection requires its own source account".into());
        }
    }
    let current = store
        .model_connection(&input.id)
        .map_err(|e| e.to_string())?;
    if current.as_ref().map(|s| s.revision) != expected {
        return Err("model_connection_revision_conflict".into());
    }
    if current.as_ref().is_some_and(|s| {
        s.document.connection.source != input.source
            || s.document.connection.source_id != input.source_id
    }) {
        return Err("model connection source identity cannot change".into());
    }
    if current.is_none()
        && store
            .list_model_profiles()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|profile| {
                serde_json::from_str::<Value>(&profile.source_json)
                    .ok()
                    .is_some_and(|source| source["endpointID"].as_str() == Some(input.id.as_str()))
            })
    {
        return Err("model connection identity already belongs to an endpoint".into());
    }
    // A source configuration has stable identity even when its address changes.
    for existing in store.list_model_connections().map_err(|e| e.to_string())? {
        let other = &existing.document.connection;
        if other.id != input.id
            && other.source == input.source
            && input.source_id.is_some()
            && other.source_id == input.source_id
        {
            return Err("model_connection_source_already_imported".into());
        }
        if other.id != input.id
            && other
                .display_name
                .trim()
                .eq_ignore_ascii_case(input.display_name.trim())
        {
            return Err("model_connection_name_conflict".into());
        }
    }
    let route_changed = current
        .as_ref()
        .is_some_and(|s| !s.document.connection.same_route(&input));
    let mut models = profiles(store, &input.id)?;
    let expected_models = revisions(&models);
    for model in &mut models {
        let mut source: Value = serde_json::from_str(&model.source_json)
            .map_err(|_| "invalid connection model source")?;
        source["baseURL"] = json!(input.base_url);
        source["endpointName"] = json!(input.display_name);
        source["endpointEnabled"] = json!(input.enabled);
        source["accountID"] = json!(input.account_id);
        source["sourceKind"] = json!(input.source);
        if route_changed {
            source["executionCapabilities"] = Value::Null;
        }
        let protocol = source["wireProtocol"]
            .as_str()
            .and_then(|v| ModelProtocol::parse(v).ok());
        if !protocol.is_some_and(|p| input.protocols.contains(&p)) {
            model.enabled = false;
            model.selected = false;
            source["modelEnabled"] = json!(false);
        }
        model.source_json = source.to_string();
        model.updated_at_ms = at;
    }
    // Route changes invalidate discovery evidence; profiles and user choices survive.
    let capability_assessments = if route_changed {
        None
    } else {
        current
            .as_ref()
            .and_then(|s| s.document.capability_assessments.clone())
    };
    let catalog = if route_changed {
        None
    } else {
        current.and_then(|s| s.document.catalog)
    };
    let document = ModelConnectionDocument {
        format_version: 1,
        connection: input,
        catalog,
        capability_assessments,
    };
    commit(store, &document, expected, &models, &expected_models, at)
}

pub(crate) fn refresh_catalog(
    store: &mut impl ModelRepository,
    id: &str,
    expected: i64,
    models: Vec<CatalogModel>,
    at: i64,
) -> Result<Value, String> {
    let mut current = load(store, id, expected)?;
    if current
        .document
        .catalog
        .as_ref()
        .is_some_and(|catalog| at < catalog.observed_at_ms)
    {
        return Err("model_catalog_observation_is_stale".into());
    }
    current.document.catalog = Some(ModelCatalog {
        observed_at_ms: at,
        source: current.document.connection.source,
        models,
    });
    // A provider listing is an observation, not authority to erase user
    // overrides or probe evidence. Route changes invalidate these in
    // save_connection; a transient omission from /models must not.
    let profiles = profiles(store, id)?;
    commit(
        store,
        &current.document,
        Some(expected),
        &profiles,
        &revisions(&profiles),
        at,
    )
}

fn profile_id(connection: &str, key: &str) -> String {
    // Length-delimited identities avoid ambiguous concatenation and retain IDs across URL edits.
    let mut digest = Sha256::new();
    digest.update((connection.len() as u64).to_be_bytes());
    digest.update(connection.as_bytes());
    digest.update(key.as_bytes());
    format!("connection-model-{:x}", digest.finalize())
}

/// Called inside the Application command transaction, including direct callers.
pub(crate) fn import_connection(
    store: &mut impl ModelRepository,
    input: ModelConnection,
    expected: Option<i64>,
    account: Option<ModelAccount>,
    expected_account: Option<i64>,
    models: Vec<CatalogModel>,
    enable: bool,
    at: i64,
) -> Result<Value, String> {
    if let Some(account) = account {
        account.validate()?;
        if input.account_id.as_deref() != Some(account.id.as_str()) || account.source != input.source {
            return Err("import_account_owner_mismatch".into());
        }
        // A source import cannot silently rotate another connection's account.
        if store.list_model_connections().map_err(|e| e.to_string())?.iter().any(|snapshot|
            snapshot.document.connection.id != input.id &&
            snapshot.document.connection.account_id.as_deref() == Some(account.id.as_str())) {
            return Err("import_account_shared_requires_separate_binding".into());
        }
        store.save_model_account(&account, expected_account, at).map_err(|e| e.to_string())?;
    } else if expected_account.is_some() {
        return Err("import_account_revision_without_account".into());
    }
    let saved: ModelConnectionSnapshot = serde_json::from_value(save_connection(store, input.clone(), expected, at)?)
        .map_err(|_| "invalid_saved_connection")?;
    let mut keys: Vec<_> = models.iter().map(CatalogModel::catalog_key).collect();
    let refreshed: ModelConnectionSnapshot = serde_json::from_value(refresh_catalog(store, &input.id, saved.revision, models, at)?)
        .map_err(|_| "invalid_saved_catalog")?;
    if !enable || keys.is_empty() { return serde_json::to_value(refreshed).map_err(|e| e.to_string()); }
    let mut selected = None;
    for profile in profiles(store, &input.id)? {
        if !profile.enabled { continue; }
        let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "invalid_stored_model_source")?;
        let key = source["catalogKey"].as_str().ok_or("model_catalog_key_missing")?.to_owned();
        if profile.selected { selected = Some(key.clone()); }
        if !keys.contains(&key) { keys.push(key); }
    }
    if selected.is_none() && expected.is_none() { selected = keys.first().cloned(); }
    configure_models(store, &input.id, refreshed.revision, &keys, selected.as_deref(), at)
}

pub(crate) fn set_capability_assessment(
    store: &mut impl ModelRepository,
    id: &str,
    expected: i64,
    model_id: &str,
    protocol: ModelProtocol,
    evidence: Option<ModelCapabilityEvidence>,
    user_override: Option<Option<crate::ModelExecutionCapabilities>>,
    at: i64,
) -> Result<Value, String> {
    let mut current = load(store, id, expected)?;
    let entry = current
        .document
        .catalog
        .as_ref()
        .and_then(|catalog| {
            catalog
                .models
                .iter()
                .find(|m| m.model_id == model_id && m.protocol == protocol)
        })
        .ok_or("model_catalog_entry_missing")?
        .clone();
    let at = if let Some(evidence) = &evidence {
        let attempt = store
            .model_probe_attempt(&evidence.request_id)
            .map_err(|e| e.to_string())?
            .ok_or("model_probe_attempt_missing")?;
        if attempt.connection_id != id
            || attempt.connection_revision != expected
            || attempt.model_id != model_id
            || attempt.protocol != protocol
            || attempt.credential_binding != evidence.credential_binding
            || attempt.account_revision != evidence.account_revision
            || attempt.finished_at_ms.is_some()
        {
            return Err("model_probe_evidence_attempt_mismatch".into());
        }
        let account_revision = if let Some(id) = &current.document.connection.account_id {
            Some(
                store
                    .model_account(id)
                    .map_err(|e| e.to_string())?
                    .ok_or("model_account_not_found")?
                    .revision,
            )
        } else {
            None
        };
        if evidence.connection_revision != expected
            || evidence.requests_sent != 1
            || evidence.account_revision != account_revision
            || evidence.credential_binding.len() != 64
            || !evidence.text_generation
            || evidence.observed_at_ms <= 0
            || evidence.request_id.is_empty()
            || evidence.request_id.len() > 128
        {
            return Err("model_capability_evidence_invalid".into());
        }
        evidence.observed_at_ms
    } else {
        at
    };
    if let Some(Some(capabilities)) = user_override {
        crate::ModelExecutionCapabilities::from_source(
            &json!({"executionCapabilities":capabilities}),
        )?;
    }
    let assessments = current
        .document
        .capability_assessments
        .get_or_insert_with(Vec::new);
    let index = assessments
        .iter()
        .position(|a| a.model_id == model_id && a.protocol == protocol);
    let assessment = if let Some(index) = index {
        &mut assessments[index]
    } else {
        assessments.push(ModelCapabilityAssessment {
            model_id: model_id.into(),
            protocol,
            evidence: None,
            user_override: None,
        });
        assessments.last_mut().expect("just inserted")
    };
    if let Some(evidence) = evidence {
        assessment.evidence = Some(evidence);
    }
    if let Some(user_override) = user_override {
        assessment.user_override = user_override;
    }
    let (capabilities, basis) = effective_capabilities(&current.document, &entry);
    let mut models = profiles(store, id)?;
    let expected_models = revisions(&models);
    for model in &mut models {
        let mut source: Value = serde_json::from_str(&model.source_json)
            .map_err(|_| "invalid connection model source")?;
        if source["catalogKey"].as_str() == Some(entry.catalog_key().as_str()) {
            source["executionCapabilities"] = json!(capabilities);
            source["executionCapabilityBasis"] = json!(basis);
            model.source_json = source.to_string();
            model.updated_at_ms = at;
        }
    }
    commit(
        store,
        &current.document,
        Some(expected),
        &models,
        &expected_models,
        at,
    )
}

fn effective_capabilities(
    document: &ModelConnectionDocument,
    entry: &CatalogModel,
) -> (crate::ModelExecutionCapabilities, &'static str) {
    if let Some(capabilities) = document
        .capability_assessments
        .as_ref()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.model_id == entry.model_id && item.protocol == entry.protocol)
        })
        .and_then(|item| item.user_override)
    {
        (capabilities, "user_override")
    } else if let Some(capabilities) = entry.execution_capabilities {
        (capabilities, "catalog_declaration")
    } else {
        (
            crate::ModelExecutionCapabilities {
                reasoning: !entry.reasoning_levels.is_empty(),
                ..crate::ModelExecutionCapabilities::agent_baseline()
            },
            if entry.reasoning_levels.is_empty() { "protocol_agent_baseline" } else { "catalog_declaration" },
        )
    }
}

fn catalog_reasoning(entry: &CatalogModel) -> Result<Value, String> {
    // ChatGPT's picker also advertises Ultra, a multi-agent execution mode.
    // Keep that source observation in the catalog; it is not a single-model
    // Responses effort and must never enter an execution profile or wire request.
    let efforts: Vec<String> = entry.reasoning_levels.iter().filter(|value| {
        !(entry.protocol == nola_domain::generation::ModelProtocol::SiwcResponses && value.as_str() == "ultra")
    }).cloned().collect();
    if efforts.is_empty() { return Ok(Value::Null); }
    let declaration = nola_domain::reasoning_tiers::ReasoningCapabilityDeclaration::Effort {
        values: efforts.clone(),
    };
    declaration.validate(entry.protocol).map_err(|error| error.to_string())?;
    Ok(json!({"control":"effort", "efforts":efforts}))
}

pub(crate) fn configure_models(
    store: &mut impl ModelRepository,
    id: &str,
    expected: i64,
    enabled: &[String],
    selected: Option<&str>,
    at: i64,
) -> Result<Value, String> {
    let current = load(store, id, expected)?;
    let connection = &current.document.connection;
    let enabled_keys: BTreeSet<&str> = enabled.iter().map(String::as_str).collect();
    if enabled_keys.len() != enabled.len()
        || selected.is_some_and(|key| !enabled_keys.contains(key))
    {
        return Err("invalid connection model selection".into());
    }
    let mut models = profiles(store, id)?;
    let expected_models = revisions(&models);
    let catalog: BTreeMap<String, &CatalogModel> = current
        .document
        .catalog
        .as_ref()
        .map(|c| c.models.iter().map(|m| (m.catalog_key(), m)).collect())
        .unwrap_or_default();
    let existing_keys: BTreeSet<String> = models
        .iter()
        .filter_map(|p| {
            serde_json::from_str::<Value>(&p.source_json)
                .ok()
                .and_then(|s| s["catalogKey"].as_str().map(str::to_owned))
        })
        .collect();
    for key in &enabled_keys {
        if !catalog.contains_key(*key) && !existing_keys.contains(*key) {
            return Err("connection model is absent from catalog and manual profiles".into());
        }
    }
    for model in &mut models {
        let mut source: Value = serde_json::from_str(&model.source_json)
            .map_err(|_| "invalid connection model source")?;
        let key = source["catalogKey"]
            .as_str()
            .ok_or("connection model catalog key missing")?
            .to_owned();
        let enabled = enabled_keys.contains(key.as_str());
        if enabled {
            let protocol = ModelProtocol::parse(
                source["wireProtocol"]
                    .as_str()
                    .ok_or("connection model protocol missing")?,
            )?;
            if !connection.protocols.contains(&protocol) {
                return Err("connection model protocol is disabled".into());
            }
            // Catalog discovery can supply a declaration only on an explicit user configuration.
            if let Some(entry) = catalog.get(&key) {
                let (capabilities, basis) = effective_capabilities(&current.document, entry);
                source["executionCapabilities"] = json!(capabilities);
                source["executionCapabilityBasis"] = json!(basis);
                if entry.summary_supported.is_some() || source["reasoningSummarySupported"].is_null() {
                    source["reasoningSummarySupported"] = json!(entry.summary_supported);
                }
                // Connection-managed declarations belong to this catalog;
                // user-selected tiers remain in parameters, not this source.
                source["reasoning"] = catalog_reasoning(entry)?;
            } else if source["executionCapabilities"].is_null() {
                // An explicitly re-enabled, retained manual model remains a
                // agent route after endpoint changes invalidate its old catalog.
                source["executionCapabilities"] =
                    json!(crate::ModelExecutionCapabilities::agent_baseline());
                source["executionCapabilityBasis"] = json!("protocol_agent_baseline");
            }
            crate::ModelExecutionCapabilities::from_source(&source)?;
        }
        model.enabled = enabled;
        model.selected = selected == Some(key.as_str());
        source["modelEnabled"] = json!(enabled);
        model.source_json = source.to_string();
        model.updated_at_ms = at;
    }
    for key in enabled_keys
        .iter()
        .filter(|key| !existing_keys.contains(**key))
    {
        let entry = catalog.get(*key).ok_or("model catalog entry disappeared")?;
        let (capabilities, basis) = effective_capabilities(&current.document, entry);
        let source = json!({"schemaVersion":2,"connectionID":id,"endpointID":id,
            "endpointName":connection.display_name,"baseURL":connection.base_url,
            "wireProtocol":entry.protocol.as_str(),"endpointEnabled":connection.enabled,
            "accountID":connection.account_id,"sourceKind":connection.source,"catalogKey":key,"modelEnabled":true,
            "endpointOrder":0,"modelOrder":models.len(),"priority":50,"capabilities":["chat"],
            "executionCapabilities":capabilities,"reasoning":catalog_reasoning(entry)?,
            "reasoningSummarySupported":entry.summary_supported,
            "executionCapabilityBasis":basis});
        models.push(StoredModelProfile {
            id: profile_id(id, key), display_name: entry.display_name.clone(),
            provider: if connection.source == ModelSourceKind::NolaChatgpt { "nola-chatgpt" } else { "openai-compatible" }.into(), model_name: entry.model_id.clone(), runtime: "remote".into(),
            source_json: source.to_string(),
            parameters_json: remote_profile_parameters(entry.context_window, entry.output_limit).to_string(),
            selected: selected == Some(*key), enabled: true, revision: 0, created_at_ms: at,
            updated_at_ms: at, deleted_at_ms: None,
        });
    }
    commit(
        store,
        &current.document,
        Some(expected),
        &models,
        &expected_models,
        at,
    )
}
