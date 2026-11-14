// SPDX-License-Identifier: Apache-2.0
//! Transactional conversation identity and profile persistence.
use super::{
    conversation_metadata_sync, params, publish_conversation_content, require_id, validate_json,
    NolaStore, StoreError, StoreResult,
};
use nola_store_port::conversation_profile::{
    CreateConversationInput, CreateConversationResult,
};

impl NolaStore {
    /// Preserve fields not owned by this edit, without a read/write race.
    pub fn patch_conversation_metadata(
        &mut self,
        input: &nola_store_port::conversation_profile::PatchConversationMetadataInput,
    ) -> StoreResult<Option<i64>> {
        use rusqlite::OptionalExtension;
        use serde_json::{json, Value};
        require_id("conversation.id", &input.conversation_id)?;
        if input.expected_revision < 1 {
            return Err(StoreError::Invariant("expected revision must be positive".into()));
        }
        let patch: Value = serde_json::from_str(&input.patch_json)?;
        nola_store_port::conversation_profile::validate_metadata_fields(&patch)
            .map_err(|error| StoreError::Invariant(error.into()))?;
        let patch = patch.as_object().filter(|fields| !fields.is_empty())
            .ok_or_else(|| StoreError::Invariant("metadata patch must be a nonempty object".into()))?;
        if patch.contains_key("modelSelectionID") || patch.contains_key("executionHostBinding") || patch.contains_key("executionHostRawValue") {
            return Err(StoreError::Invariant("modelSelectionID must use the conversation configuration command".into()));
        }
        let transaction = self.write_transaction()?;
        let current: Option<(i64, Option<String>)> = transaction.query_row(
            "SELECT c.revision,s.settings_json FROM conversations c LEFT JOIN conversation_settings s ON s.conversation_id=c.id WHERE c.id=?1",
            params![input.conversation_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((revision, settings)) = current.filter(|(revision, _)| *revision == input.expected_revision) else {
            transaction.commit()?;
            return Ok(None);
        };
        let next_revision = revision.checked_add(1)
            .ok_or_else(|| StoreError::Invariant("conversation revision overflow".into()))?;
        let mut settings: Value = match settings {
            Some(raw) => serde_json::from_str(&raw)?,
            None => json!({}),
        };
        let fields = settings.as_object_mut()
            .ok_or_else(|| StoreError::Invariant("conversation settings must be an object".into()))?;
        for (key, value) in patch {
            if value.is_null() {
                fields.remove(key);
            } else {
                fields.insert(key.clone(), value.clone());
            }
        }
        let settings_json = serde_json::to_string(&settings)?;
        transaction.execute(
            "UPDATE conversations SET revision=?2,updated_at_ms=?3 WHERE id=?1",
            params![input.conversation_id, next_revision, input.updated_at_ms],
        )?;
        transaction.execute("INSERT INTO conversation_settings(conversation_id,settings_json,updated_at_ms) VALUES(?1,?2,?3) ON CONFLICT(conversation_id) DO UPDATE SET settings_json=excluded.settings_json,updated_at_ms=excluded.updated_at_ms", params![input.conversation_id,settings_json,input.updated_at_ms])?;
        conversation_metadata_sync::enqueue(&transaction, &input.conversation_id)?;
        transaction.commit()?;
        Ok(Some(next_revision))
    }

    /// Field-owned merge and revision comparison occur in the same transaction.
    /// Reuses native modelOverrides / permissionModeRawValue, never a Web copy.
    pub fn patch_conversation_configuration(
        &mut self,
        input: &nola_store_port::conversation_profile::PatchConversationConfigurationInput,
    ) -> StoreResult<nola_domain::conversation_configuration::ConfigurationCommitResult> {
        use nola_domain::conversation_configuration::{ConfigurationCommitResult, ConversationConfigurationSnapshot};
        use rusqlite::OptionalExtension;
        use serde_json::{json, Value};
        require_id("conversation.id", &input.conversation_id)?;
        if input.expected_revision < 1 {
            return Err(StoreError::Invariant(
                "expected revision must be positive".into(),
            ));
        }
        let patch = input.patch_json.fields();
        let transaction = self.write_transaction()?;
        let current: Option<(i64, Option<String>)> = transaction.query_row(
            "SELECT c.configuration_revision,s.settings_json FROM conversations c LEFT JOIN conversation_settings s ON s.conversation_id=c.id WHERE c.id=?1",
            params![input.conversation_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let Some((revision, settings)) = current else {
            transaction.commit()?;
            return Ok(ConfigurationCommitResult::NotFound { conversation_id: input.conversation_id.clone() });
        };
        if revision != input.expected_revision {
            transaction.commit()?;
            return Ok(ConfigurationCommitResult::Conflict { conversation_id: input.conversation_id.clone(), expected_revision: input.expected_revision, actual_revision: revision });
        }
        let next_revision = revision.checked_add(1).ok_or_else(|| StoreError::Invariant("configuration revision overflow".into()))?;
        let mut settings: Value = match settings {
            Some(raw) => serde_json::from_str(&raw)?,
            None => json!({}),
        };
        let fields = settings.as_object_mut().ok_or_else(|| {
            StoreError::Invariant("conversation settings must be an object".into())
        })?;
        for key in ["language", "selectedPromptTemplateID", "expertReasoningEffort", "promptResponseStyle", "executionBackend", "persistenceEnabled", "dataSourceIds", "executionPolicy", "promptTemplateSelectionIsExplicit", "contextLimitOverride"] {
            if let Some(value) = patch.get(key) {
                if value.is_null() { fields.remove(key); } else { fields.insert(key.into(), value.clone()); }
            }
        }
        for key in ["dataSourceOverrides", "externalExecutionSessions", "externalExecutionModelIDs", "externalExecutionModelProviderIDs"] {
            if let Some(updates) = patch.get(key) {
                let updates = updates.as_object().ok_or_else(|| StoreError::Invariant(format!("{key} must be an object")))?;
                let stored = fields.entry(key).or_insert_with(|| json!({})).as_object_mut()
                    .ok_or_else(|| StoreError::Invariant(format!("stored {key} must be an object")))?;
                for (key, value) in updates {
                    if value.is_null() { stored.remove(key); } else { stored.insert(key.clone(), value.clone()); }
                }
            }
        }
        if let Some(knowledge) = patch.get("knowledge") {
            let patch = nola_store_port::conversation_profile::knowledge_patch_fields(knowledge)
                .map_err(|error| StoreError::Invariant(error.into()))?;
            let stored = fields.entry("knowledge").or_insert(Value::Null);
            if stored.is_null() {
                *stored = json!({"keyPointsMarkdown":"", "wikiMarkdown":"", "updatedAt":null});
            }
            let stored = stored.as_object_mut().ok_or_else(|| StoreError::Invariant("stored knowledge must be an object".into()))?;
            if !["keyPointsMarkdown", "wikiMarkdown"].iter().all(|key| stored.get(*key).is_some_and(Value::is_string))
                || stored.get("updatedAt").is_some_and(|value| !value.is_null() && !value.is_number()) {
                return Err(StoreError::Invariant("invalid stored knowledge document".into()));
            }
            for (key, value) in patch { stored.insert(key.clone(), value.clone()); }
            // MacConversationKnowledge uses Foundation's default Codable Date:
            // seconds since 2001-01-01, not Unix seconds or an ISO string.
            stored.insert("updatedAt".into(), json!(input.updated_at_ms as f64 / 1000.0 - 978_307_200.0));
        }
        for key in ["memoryEnabledOverride", "retrievalEnabledOverride"] {
            if let Some(value) = patch.get(key) {
                if value.is_null() {
                    fields.remove(key);
                } else if value.is_boolean() {
                    fields.insert(key.into(), value.clone());
                } else {
                    return Err(StoreError::Invariant(format!("{key} must be boolean or null")));
                }
            }
        }
        if let Some(pinned) = patch.get("isPinned") {
            if !pinned.is_boolean() {
                return Err(StoreError::Invariant("isPinned must be a boolean".into()));
            }
            fields.insert("isPinned".into(), pinned.clone());
        }
        if let Some(permission) = patch.get("permissionProfile") {
            if permission.is_null() {
                fields.remove("permissionModeRawValue");
            } else {
                fields.insert("permissionModeRawValue".into(), permission.clone());
            }
        }
        if let Some(models) = patch.get("modelOverrides") {
            let models = models
                .as_object()
                .ok_or_else(|| StoreError::Invariant("modelOverrides must be an object".into()))?;
            let stored = fields
                .entry("modelOverrides")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| {
                    StoreError::Invariant("stored modelOverrides must be an object".into())
                })?;
            for (id, patch) in models {
                if patch.is_null() {
                    stored.remove(id);
                    continue;
                }
                let patch = patch.as_object().ok_or_else(|| {
                    StoreError::Invariant("model override must be an object".into())
                })?;
                let model = stored
                    .entry(id.clone())
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or_else(|| {
                        StoreError::Invariant("stored model override must be an object".into())
                    })?;
                for (key, value) in patch {
                    if value.is_null() {
                        model.remove(key);
                    } else {
                        model.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        if let Some(models) = patch.get("clientModelPresentation") {
            let models = models.as_object().ok_or_else(|| StoreError::Invariant("model presentation must be an object".into()))?;
            let stored = fields.entry("clientModelPresentation").or_insert_with(|| json!({})).as_object_mut()
                .ok_or_else(|| StoreError::Invariant("stored model presentation must be an object".into()))?;
            for (key, value) in models {
                if value.is_null() { stored.remove(key); continue; }
                let updates = value.as_object().ok_or_else(|| StoreError::Invariant("model presentation must be an object".into()))?;
                let model = stored.entry(key.clone()).or_insert_with(|| json!({})).as_object_mut()
                    .ok_or_else(|| StoreError::Invariant("stored model presentation must be an object".into()))?;
                for (field, value) in updates {
                    if value.is_null() { model.remove(field); } else { model.insert(field.clone(), value.clone()); }
                }
            }
        }
        // Explicit parameters must participate in the same first-selection
        // transaction; defaults fill only fields still absent after merging.
        for key in ["executionHostRawValue", "executionHostBinding", "clientHostParameters"] {
            if let Some(value) = patch.get(key) {
                if key == "executionHostRawValue" && !value.is_null() && !value.as_str().is_some_and(|value| ["local", "remote"].contains(&value)) {
                    return Err(StoreError::Invariant("invalid conversation execution host".into()));
                }
                if value.is_null() { fields.remove(key); } else { fields.insert(key.into(), value.clone()); }
            }
        }
        if let Some(selection) = patch.get("modelSelectionID") {
            if selection.is_null() { fields.remove("modelSelectionID"); }
            else {
                if !patch.get("executionHostBinding").is_some_and(|value| !value.is_null()) {
                    fields.remove("executionHostBinding");
                    fields.remove("clientHostParameters");
                }
                initialize_selected_model(&transaction, fields, selection)?;
            }
        }
        if fields.get("modelSelectionID").is_some_and(|value| !value.is_null())
            && fields.get("executionHostBinding").is_some_and(|value| !value.is_null()) {
            return Err(StoreError::Invariant("model selection and execution host binding are mutually exclusive".into()));
        }
        let settings_json = serde_json::to_string(&settings)?;
        transaction.execute(
            "UPDATE conversations SET configuration_revision=configuration_revision+1,revision=revision+1,updated_at_ms=?2 WHERE id=?1",
            params![input.conversation_id, input.updated_at_ms],
        )?;
        transaction.execute("INSERT INTO conversation_settings(conversation_id,settings_json,updated_at_ms) VALUES(?1,?2,?3) ON CONFLICT(conversation_id) DO UPDATE SET settings_json=excluded.settings_json,updated_at_ms=excluded.updated_at_ms", params![input.conversation_id,settings_json,input.updated_at_ms])?;
        conversation_metadata_sync::enqueue(&transaction, &input.conversation_id)?;
        let field = |key: &str| settings.get(key).cloned().unwrap_or(Value::Null);
        let snapshot = ConversationConfigurationSnapshot {
            conversation_id: input.conversation_id.clone(),
            configuration_revision: next_revision,
            execution_host_raw_value: field("executionHostRawValue"),
            execution_host_binding: field("executionHostBinding"),
            client_host_parameters: field("clientHostParameters"),
            client_model_presentation: settings.get("clientModelPresentation").cloned().unwrap_or(json!({})),
            model_selection_id: field("modelSelectionID"), knowledge: field("knowledge"),
            retrieval_enabled_override: field("retrievalEnabledOverride"),
            memory_enabled_override: field("memoryEnabledOverride"),
            is_pinned: settings.get("isPinned").and_then(Value::as_bool).unwrap_or(false),
            model_overrides: settings.get("modelOverrides").cloned().unwrap_or(json!({})),
            permission_profile: field("permissionModeRawValue"),
            language: field("language"), selected_prompt_template_id: field("selectedPromptTemplateID"),
            data_source_overrides: field("dataSourceOverrides"), data_source_ids: field("dataSourceIds"),
            execution_backend: field("executionBackend"), external_execution_sessions: field("externalExecutionSessions"),
            persistence_enabled: field("persistenceEnabled"), prompt_response_style: field("promptResponseStyle"),
            prompt_template_selection_is_explicit: field("promptTemplateSelectionIsExplicit"),
            context_limit_override: field("contextLimitOverride"),
            external_execution_model_ids: field("externalExecutionModelIDs"),
            external_execution_model_provider_ids: field("externalExecutionModelProviderIDs"),
            execution_policy: settings.get("executionPolicy").filter(|value| !value.is_null())
                .cloned().map(serde_json::from_value).transpose()?,
        };
        transaction.commit()?;
        Ok(ConfigurationCommitResult::Committed { snapshot })
    }

    pub fn create_conversation(
        &mut self,
        input: &CreateConversationInput,
    ) -> StoreResult<CreateConversationResult> {
        require_id("conversation.id", &input.id)?;
        if let Some(settings) = &input.settings_json {
            validate_json("conversation.settings_json", settings)?;
        }
        let transaction = self.write_transaction()?;
        let exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1)",
            params![input.id],
            |row| row.get::<_, bool>(0),
        )?;
        if exists {
            transaction.commit()?;
            return Ok(CreateConversationResult { created: false, initial_configuration_revision: None });
        }
        let settings_json = input.settings_json.as_deref().map(|raw| initialize_settings_model(&transaction, raw)).transpose()?;

        transaction.execute(
            "INSERT INTO conversations(id, title, revision, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                input.id,
                input.title,
                input.revision.max(1),
                input.created_at_ms,
                input.updated_at_ms
            ],
        )?;
        if let Some(settings) = &settings_json {
            transaction.execute(
                "INSERT INTO conversation_settings(conversation_id, settings_json, updated_at_ms)
                 VALUES (?1, ?2, ?3)",
                params![input.id, settings, input.updated_at_ms],
            )?;
        }
        publish_conversation_content(&transaction, &input.id, input.updated_at_ms)?;
        let initial_configuration_revision = transaction.query_row(
            "SELECT configuration_revision FROM conversations WHERE id=?1", [&input.id], |row| row.get::<_, i64>(0))?;
        transaction.commit()?;
        Ok(CreateConversationResult { created: true, initial_configuration_revision: Some(initial_configuration_revision) })
    }

    pub fn rename_conversation(
        &mut self,
        conversation_id: &str,
        title: &str,
        title_source: Option<nola_domain::conversation_configuration::ConversationTitleSource>,
        updated_at_ms: i64,
    ) -> StoreResult<bool> {
        if title.trim().is_empty() {
            return Err(StoreError::Invariant(
                "conversation title must not be empty".to_string(),
            ));
        }
        let transaction = self.write_transaction()?;
        let updated = transaction.execute(
            "UPDATE conversations SET title = ?2, revision = revision + 1, updated_at_ms = ?3
             WHERE id = ?1",
            params![conversation_id, title, updated_at_ms],
        )?;
        if updated == 0 {
            transaction.rollback()?;
            return Ok(false);
        }
        if let Some(source)=title_source {
            use rusqlite::OptionalExtension;
            let raw:Option<String>=transaction.query_row("SELECT settings_json FROM conversation_settings WHERE conversation_id=?1",[conversation_id],|row|row.get(0)).optional()?;
            let mut settings:serde_json::Value=serde_json::from_str(raw.as_deref().unwrap_or("{}"))?;
            settings.as_object_mut().ok_or_else(||StoreError::Invariant("settings must be an object".into()))?.insert("titleSource".into(),serde_json::to_value(source)?);
            transaction.execute("INSERT INTO conversation_settings(conversation_id,settings_json,updated_at_ms) VALUES(?1,?2,?3) ON CONFLICT(conversation_id) DO UPDATE SET settings_json=excluded.settings_json,updated_at_ms=excluded.updated_at_ms",params![conversation_id,serde_json::to_string(&settings)?,updated_at_ms])?;
        }
        conversation_metadata_sync::enqueue(&transaction, conversation_id)?;
        transaction.commit()?;
        Ok(true)
    }

}

fn initialize_settings_model(transaction: &rusqlite::Connection, raw: &str) -> StoreResult<String> {
    let mut value: serde_json::Value = serde_json::from_str(raw)?;
    if let Some(selection) = value.get("modelSelectionID").filter(|value| !value.is_null()).cloned() {
        let fields = value.as_object_mut().ok_or_else(|| StoreError::Invariant("conversation settings must be an object".into()))?;
        initialize_selected_model(transaction, fields, &selection)?;
    }
    Ok(serde_json::to_string(&value)?)
}

/// Model selection and the first parameter snapshot are one store transaction.
/// Re-selecting a model preserves this conversation's prior explicit settings.
fn initialize_selected_model(
    transaction: &rusqlite::Connection,
    settings: &mut serde_json::Map<String, serde_json::Value>,
    selection: &serde_json::Value,
) -> StoreResult<()> {
    use rusqlite::OptionalExtension;
    use serde_json::{json, Value};
    let key = selection.as_str().ok_or_else(|| StoreError::Invariant("model selection must be explicit".into()))?;
    if let Some(id) = key.strip_prefix("configured:") {
        let profile: Option<(String, String)> = transaction.query_row(
            "SELECT parameters_json, source_json FROM model_profiles WHERE id=?1 AND enabled=1 AND deleted_at_ms IS NULL",
            params![id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let (parameters, source) = profile.ok_or_else(|| StoreError::Invariant("selected model is unavailable".into()))?;
        let parameters: Value = serde_json::from_str(&parameters)?;
        let source: Value = serde_json::from_str(&source)?;
        if ["endpointEnabled", "modelEnabled"].iter().any(|key| source.get(*key).and_then(Value::as_bool) == Some(false)) {
            return Err(StoreError::Invariant("selected model is unavailable".into()));
        }
        let models = settings.entry("modelOverrides").or_insert_with(|| json!({})).as_object_mut()
            .ok_or_else(|| StoreError::Invariant("model overrides must be an object".into()))?;
        let model = models.entry(key).or_insert_with(|| json!({})).as_object_mut()
            .ok_or_else(|| StoreError::Invariant("model override must be an object".into()))?;
        // Profile defaults must not manufacture an explicit sampling request
        // for a route that cannot send sampling parameters.
        let execution = &source["executionCapabilities"];
        if execution["samplingParameters"] == true && execution["samplingRequiresNoReasoning"] != true {
            for (field, default) in [("temperature", json!(0.7)), ("topP", json!(0.9))] {
                model.entry(field).or_insert_with(|| parameters.get(field).cloned().unwrap_or(default));
            }
        }
        for (field, default) in [
            ("maxOutputTokens", json!(16384)), ("maxContextLength", json!(128000)),
            ("topK", Value::Null), ("repetitionPenalty", Value::Null),
            ("stopSequences", json!(""))] {
            model.entry(field).or_insert_with(|| parameters.get(field).cloned().unwrap_or(default));
        }
        for field in ["presencePenalty", "frequencyPenalty"] {
            model.entry(field).or_insert_with(|| parameters.get(field).filter(|value| value.as_f64() != Some(0.0)).cloned().unwrap_or(Value::Null));
        }
        let default_seed=match parameters.get("seed").filter(|value|!value.is_null()) {
            None=>json!({"kind":"clear"}),
            Some(value)=>json!({"kind":"value","value":value.as_i64().ok_or_else(||StoreError::Invariant("model profile seed must be an int64".into()))?.to_string()}),
        };
        model.entry("seed").or_insert(default_seed);
        model.entry("reasoningPreference").or_insert_with(|| source["reasoning"].get("defaultPreference").cloned().unwrap_or(json!("automatic")));
    } else if !key.strip_prefix("local:").is_some_and(|id| !id.trim().is_empty()) && !key.starts_with("connectedHost:") {
        return Err(StoreError::Invariant("invalid model selection".into()));
    }
    settings.insert("modelSelectionID".into(), selection.clone());
    Ok(())
}

#[cfg(test)]
mod configuration_initialization_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selected_profile_only_materializes_supported_sampling_defaults() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE model_profiles(id TEXT PRIMARY KEY, parameters_json TEXT, source_json TEXT, enabled INTEGER, deleted_at_ms INTEGER);").unwrap();
        connection.execute("INSERT INTO model_profiles VALUES('model','{}','{}',1,NULL)", []).unwrap();
        for (supported, constrained, expected) in [(false, false, false), (true, false, true), (true, true, false)] {
            connection.execute("UPDATE model_profiles SET source_json=?1", [json!({"executionCapabilities":{
                "toolCalls":true,"imageInput":false,"reasoning":constrained,"samplingParameters":supported,"samplingRequiresNoReasoning":constrained}}).to_string()]).unwrap();
            let mut settings = serde_json::Map::new();
            initialize_selected_model(&connection, &mut settings, &json!("configured:model")).unwrap();
            let model = &settings["modelOverrides"]["configured:model"];
            assert_eq!(model.get("temperature").is_some(), expected);
            assert_eq!(model.get("topP").is_some(), expected);
            settings.get_mut("modelOverrides").unwrap()["configured:model"]["temperature"] = json!(0.2);
            initialize_selected_model(&connection, &mut settings, &json!("configured:model")).unwrap();
            assert_eq!(settings["modelOverrides"]["configured:model"]["temperature"], 0.2);
        }
    }

    #[test]
    fn catalog_seed_has_one_current_representation_and_override_intent_is_explicit() {
        let connection=rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE model_profiles(id TEXT PRIMARY KEY, parameters_json TEXT, source_json TEXT, enabled INTEGER, deleted_at_ms INTEGER);").unwrap();
        connection.execute("INSERT INTO model_profiles VALUES('seed','{}','{}',1,NULL)",[]).unwrap();
        for seed in [json!(0),json!(i64::MIN),json!(i64::MAX)] {
            connection.execute("UPDATE model_profiles SET parameters_json=?1",[json!({"seed":seed}).to_string()]).unwrap();
            let mut settings=serde_json::Map::new();
            initialize_selected_model(&connection,&mut settings,&json!("configured:seed")).unwrap();
            assert_eq!(settings["modelOverrides"]["configured:seed"]["seed"],json!({"kind":"value","value":seed.as_i64().unwrap().to_string()}));
        }
        for seed in [json!({"kind":"clear"}),json!({"kind":"value","value":"0"}),json!("0"),json!(false)] {
            connection.execute("UPDATE model_profiles SET parameters_json=?1",[json!({"seed":seed}).to_string()]).unwrap();
            assert!(initialize_selected_model(&connection,&mut serde_json::Map::new(),&json!("configured:seed")).is_err());
        }
    }

     #[test]
    fn execution_target_and_presentation_commit_atomically_with_revision() {
        use nola_store_port::conversation_profile::PatchConversationConfigurationInput;
        let mut store = NolaStore::open(":memory:").unwrap();
        store.create_conversation(&CreateConversationInput {
            id: "target-test".into(), title: "Target".into(), revision: 1,
            settings_json: Some(json!({"executionHostBinding":{"baseURL":"https://host.example"},"clientHostParameters":{"temperature":0.3},"clientModelPresentation":{"execution-host":{"showThinking":true}}}).to_string()),
            created_at_ms: 1, updated_at_ms: 1,
        }).unwrap();
        let request = |revision, patch: serde_json::Value| PatchConversationConfigurationInput {
            conversation_id: "target-test".into(), expected_revision: revision, patch_json: patch.to_string().parse().unwrap(), updated_at_ms: 2,
        };
        let receipt = store.patch_conversation_configuration(&request(1, json!({"modelSelectionID":"local:test", "executionHostRawValue":"remote", "clientModelPresentation":{"local:test":{"streamOutput":true}}}))).unwrap().into_committed().unwrap();
        let receipt: serde_json::Value = serde_json::to_value(&receipt).unwrap();
        assert_eq!(receipt["configurationRevision"], 2);
        assert_eq!(receipt["executionHostRawValue"], "remote");
        assert!(receipt["executionHostBinding"].is_null());
        assert!(receipt["clientHostParameters"].is_null());
        assert_eq!(receipt["clientModelPresentation"]["execution-host"]["showThinking"], true);
        assert_eq!(receipt["clientModelPresentation"]["local:test"]["streamOutput"], true);
        assert!(store.patch_conversation_configuration(&request(1, json!({"modelSelectionID":"local:stale"}))).unwrap().is_conflict());
        assert!(store.patch_conversation_configuration(&request(2, json!({"modelSelectionID":"configured:missing","clientModelPresentation":{"local:test":{"streamOutput":false}}}))).is_err());
        // Failed selection rolled back: the original revision remains usable.
        let receipt = store.patch_conversation_configuration(&request(2, json!({"isPinned":true}))).unwrap().into_committed().unwrap();
        let receipt: serde_json::Value = serde_json::to_value(&receipt).unwrap();
        assert_eq!(receipt["modelSelectionID"], "local:test");
        assert_eq!(receipt["executionHostRawValue"], "remote");
        assert_eq!(receipt["clientModelPresentation"]["local:test"]["streamOutput"], true);
    }

    #[test]
    fn selecting_a_model_freezes_defaults_and_reselection_preserves_conversation_values() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE model_profiles(id TEXT PRIMARY KEY, parameters_json TEXT, source_json TEXT, enabled INTEGER, deleted_at_ms INTEGER);").unwrap();
        connection.execute("INSERT INTO model_profiles VALUES('one',?1,?2,1,NULL)", params![json!({"temperature":0.3,"maxOutputTokens":8192}).to_string(), json!({"apiKey":"never-copy","reasoning":{},"executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":true}}).to_string()]).unwrap();
        let mut settings = serde_json::Map::new();
        initialize_selected_model(&connection, &mut settings, &json!("configured:one")).unwrap();
        assert_eq!(settings["modelOverrides"]["configured:one"]["temperature"], 0.3);
        assert!(!serde_json::to_string(&settings).unwrap().contains("never-copy"));
        connection.execute("UPDATE model_profiles SET parameters_json=?1", params![json!({"temperature":1.8}).to_string()]).unwrap();
        initialize_selected_model(&connection, &mut settings, &json!("configured:one")).unwrap();
        assert_eq!(settings["modelOverrides"]["configured:one"]["temperature"], 0.3);
        assert!(initialize_selected_model(&connection, &mut settings, &json!("configured:missing")).is_err());
        assert_eq!(settings["modelSelectionID"], "configured:one");
        // A complete conversation snapshot is not proof that a profile is available.
        connection.execute("UPDATE model_profiles SET enabled=0", []).unwrap();
        assert!(initialize_selected_model(&connection, &mut settings, &json!("configured:one")).is_err());
    }
}
