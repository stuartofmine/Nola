use super::{now_string, NolaStore};
use nola_application::{
    ApplicationCommand, Conversation, CreateConversation, CreateConversationBranch,
    DeleteConversationMessage, EditConversationMessage,
};
use nola_protocol::CanonicalBranchTarget;
use nola_protocol::{
    TranscriptMessageInput as MessageInput, TranscriptPartInput as MessagePartInput,
};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug)]
pub struct WebMessageWindow {
    pub assistant_journal_heads: Vec<nola_domain::assistant_journal::AssistantJournalCursor>,
    pub messages: Vec<Value>,
    pub projection: Value,
    pub projection_revision: u64,
    pub event_sequence: u64,
    pub total_count: usize,
    pub before_ordinal: Option<i64>,
    pub has_more: bool,
}

#[derive(serde::Deserialize)]
struct ConfigurationMetadata { revision:i64, configuration_revision:i64, settings_json:Option<String> }

fn configuration_metadata(app:&mut crate::application_port::CliApplicationPort,id:&str)->Result<Option<ConfigurationMetadata>,String> {
    app.execute(ApplicationCommand::LoadConversationMetadata {conversation_id:id.into()})
}

fn historical_or_plain_display(part: &Value) -> Result<String, String> {
    match part.get("kind").and_then(Value::as_str) {
        Some("text") => Ok(part
            .get("markdown")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .into()),
        Some("historical") => {
            let projection = part
                .get("projection")
                .ok_or("Missing historical projection")?;
            let kind = projection["canonical_kind"]
                .as_str()
                .ok_or("Missing historical kind")?;
            let status = projection["status"]["state"]
                .as_str()
                .ok_or("Missing historical status")?;
            if !matches!(
                status,
                "supported" | "deferred" | "unsupported" | "not_loaded"
            ) {
                return Err("Unknown historical status".into());
            }
            if kind == "reasoning" {
                return Ok(String::new());
            }
            let names: &[&str] = match (projection["schema"].as_str(), kind) {
                (Some("agent_part"), "text") => &["text"],
                (Some("message_part_view"), "text") => &["markdown"],
                (Some("agent_part"), "reasoning") => &["detail"],
                (Some("message_part_view"), "reasoning") => &["summary"],
                _ => &[],
            };
            if status != "unsupported" && status != "not_loaded" {
                for name in names {
                    let field = &projection["fields"][name];
                    if field["availability"] == "complete" {
                        if let Some(text) = field["value"].as_str() {
                            return Ok(text.into());
                        }
                    }
                }
            }
            // Status, schema and decoder codes belong to the explicit
            // historical-detail surface. The chat timeline only presents
            // canonical user content and must not leak wire diagnostics.
            Ok(projection["canonical_text"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string())
        }
        _ => Ok(String::new()),
    }
}

fn canonical_web_profile(
    conversation: &Value,
    model_selection: Option<Option<&str>>,
) -> Result<(String, String, String, i64, i64), String> {
    // Reject incomplete owned metadata before it can poison future reads.
    serde_json::from_value::<super::WebConversation>(conversation.clone())
        .map_err(|error| format!("invalid Web conversation: {error}"))?;
    if let Some(selection) = model_selection {
        if web_model_selection(conversation)?.as_deref() != selection {
            return Err("Web and canonical model selection disagree".into());
        }
    }
    let id = required_web_id(conversation, "conversationId", "conversation")?;
    let title = web_string(conversation, "title").unwrap_or_default();
    let created_at = non_empty_web_string(conversation, "createdAt").unwrap_or_else(now_string);
    let updated_at =
        non_empty_web_string(conversation, "updatedAt").unwrap_or_else(|| created_at.clone());
    let settings_view:super::WebConversation=serde_json::from_value(conversation.clone()).map_err(|error|error.to_string())?;
    let mut settings = serde_json::json!({
        "titleSource":settings_view.title_source,
        "sessionMemory":settings_view.session_memory,
        "knowledge":{"keyPointsMarkdown":settings_view.knowledge_points,"wikiMarkdown":settings_view.wiki,"updatedAt":null},
        "lastCompactedAt":settings_view.last_compacted_at,
        "executionBackend":settings_view.execution_backend,
        "externalExecutionSessions":settings_view.external_execution_sessions,
        "isPinned":settings_view.is_pinned,
    });
    if let Some(selection) = model_selection {
        settings["modelSelectionID"] = selection.map(Value::from).unwrap_or(Value::Null);
    }
    let settings_json = serde_json::to_string(&settings)
        .map_err(|error| format!("serialize canonical Web settings failed: {error}"))?;
    Ok((
        id,
        title,
        settings_json,
        web_time_millis(&created_at),
        web_time_millis(&updated_at),
    ))
}

pub(crate) fn web_model_selection(conversation: &Value) -> Result<Option<String>, String> {
    let source = conversation["selectedModelSource"]
        .as_str()
        .ok_or("missing Web model source")?;
    let id = conversation["selectedModelId"]
        .as_str()
        .ok_or("missing Web model ID")?;
    match source {
        "auto" if id.is_empty() => Ok(None),
        "cloud" if !id.trim().is_empty() => Ok(Some(format!("configured:{id}"))),
        "local" if !id.trim().is_empty() => Ok(Some(format!("local:{id}"))),
        "connected_host" if nola_domain::configuration_patch::valid_model_identity(id) && id.starts_with("connectedHost:") => Ok(Some(id.into())),
        _ => Err("invalid Web model selection".into()),
    }
}

fn project_application_conversation_state(
    conversation: &mut Value,
    usage: Option<&Value>,
    message_count: i64,
) {
    let Some(object) = conversation.as_object_mut() else {
        return;
    };
    object.insert("messageCount".to_string(), Value::from(message_count));
    let context_usage = usage
        // A projection without a model window is not a recorded request usage.
        // Keep absence distinct from a real zero-token request with a known window.
        .filter(|value| {
            value
                .get("context_window_tokens")
                .and_then(Value::as_u64)
                .is_some_and(|window| window > 0)
        })
        .cloned()
        .unwrap_or(Value::Null);
    // Runtime state belongs to the Application projection. Never trust the
    // Web-owned profile copy: a stale browser payload must not overwrite or
    // impersonate the canonical context snapshot.
    object.insert("lastContextUsage".to_string(), context_usage);
}

impl NolaStore {
    pub fn load_web_conversations(&self) -> Result<Vec<Value>, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let summaries: Vec<nola_application::ConversationSummary> =
            canonical.list_conversations()?;
        let mut out = Vec::new();
        for summary in summaries {
            let usage = canonical
                .load_conversation_context_usage_v1(&summary.id)
                .ok();
            let Some(metadata)=configuration_metadata(&mut canonical,&summary.id)? else { continue; };
            if super::web_conversation::is_agent_context(metadata.settings_json.as_deref())? { continue; }
            let mut conversation = serde_json::to_value(super::web_conversation::assemble(
                &summary,
                metadata.settings_json.as_deref(),
                metadata.configuration_revision,
            )?)
            .map_err(|error| error.to_string())?;
            project_application_conversation_state(
                &mut conversation,
                usage.as_ref(),
                summary.message_count,
            );
            out.push(conversation);
        }
        Ok(out)
    }

    pub fn load_web_messages(&self) -> Result<Vec<Value>, String> {
        let conversations = self.load_web_conversations()?;
        let mut out = Vec::new();
        for conversation in conversations {
            let conversation_id = required_web_id(&conversation, "conversationId", "conversation")?;
            out.extend(canonical_web_messages(self, &conversation_id)?.unwrap_or_default());
        }
        Ok(out)
    }

    pub fn get_web_conversation(&self, conversation_id: &str) -> Result<Option<Value>, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let Some(summary) = canonical.conversation_summary(conversation_id)? else {
            return Ok(None);
        };
        let Some(metadata)=configuration_metadata(&mut canonical,conversation_id)? else { return Ok(None); };
        let mut conversation = serde_json::to_value(super::web_conversation::assemble(
            &summary,
            metadata.settings_json.as_deref(),
            metadata.configuration_revision,
        )?)
        .map_err(|error| error.to_string())?;
        let usage = canonical
            .load_conversation_context_usage_v1(conversation_id)
            .ok();
        project_application_conversation_state(
            &mut conversation,
            usage.as_ref(),
            summary.message_count,
        );
        Ok(Some(conversation))
    }

    pub fn web_automatic_title_is_current(&self, conversation_id: &str) -> Result<bool, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let Some(summary) = canonical.conversation_summary(conversation_id)? else {
            return Ok(false);
        };
        let settings = canonical.load_conversation_settings(conversation_id)?;
        let settings:Value=serde_json::from_str(settings.as_deref().unwrap_or("{}")).map_err(|error|error.to_string())?;
        Ok(!summary.title.is_empty() && settings.get("titleSource").and_then(Value::as_str)==Some("automatic"))
    }

    pub fn web_conversation_revision(&self, conversation_id: &str) -> Result<i64, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        canonical
            .load_conversation(conversation_id)?
            .map(|conversation| conversation.revision)
            .ok_or_else(|| format!("web conversation {conversation_id} does not exist"))
    }

    pub fn create_web_conversation(&self, conversation: &Value) -> Result<(), String> {
        let selection = web_model_selection(conversation)?;
        let (id, title, settings_json, created_at_ms, updated_at_ms) =
            canonical_web_profile(conversation, selection.as_deref().map(Some))?;
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let settings_json = initial_web_configuration(self, &mut canonical, &settings_json)?;
        canonical.create_conversation(CreateConversation {
            id,
            title,
            settings_json: Some(settings_json),
            created_at_ms,
            updated_at_ms,
        })
    }

    pub fn patch_web_conversation_configuration(
        &self,
        conversation_id: &str,
        expected_revision: i64,
        patch: Value,
    ) -> Result<nola_domain::conversation_configuration::ConfigurationCommitResult, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        canonical.execute(ApplicationCommand::PatchConversationConfiguration {
            input:nola_application::conversation_configuration::PatchConversationConfigurationInput {
                conversation_id:conversation_id.into(),expected_revision,
                patch_json:patch.to_string().parse()?,updated_at_ms:chrono::Utc::now().timestamp_millis(),
            }
        })
    }

    pub fn rename_web_conversation(&self,id:&str,title:&str,source:super::ConversationTitleSource,expected_revision:i64)->Result<bool,String> {
        crate::application_port::CliApplicationPort::persistent(self.path())?.execute_with_revision(
            u64::try_from(expected_revision).map_err(|_|"invalid title revision")?,
            ApplicationCommand::RenameConversation {conversation_id:id.into(),title:title.into(),title_source:Some(source),updated_at_ms:chrono::Utc::now().timestamp_millis()},
        )
    }

    pub fn resolve_default_web_model_selection(&self)->Result<Option<String>,String> {
        let mut app=crate::application_port::CliApplicationPort::persistent(self.path())?;
        let settings=initial_web_configuration(self,&mut app,"{}")?;
        let settings:Value=serde_json::from_str(&settings).map_err(|error|error.to_string())?;
        Ok(settings["modelSelectionID"].as_str().map(str::to_owned))
    }

    pub fn web_session_memory_edit_state(&self,id:&str)->Result<Value,String> {
        use nola_application::ContextCommand;
        use nola_store_port::context::ContextCompactionSource;
        let mut app=crate::application_port::CliApplicationPort::persistent(self.path())?;
        let source:ContextCompactionSource=app.execute(ApplicationCommand::Context {command:ContextCommand::PrepareCompaction {conversation_id:id.into()}})?;
        let metadata=configuration_metadata(&mut app,id)?.ok_or("conversation not found")?;
        let verified:ContextCompactionSource=app.execute(ApplicationCommand::Context {command:ContextCommand::PrepareCompaction {conversation_id:id.into()}})?;
        if source!=verified {return Err("context changed while reading; refresh before editing".into());}
        let settings:Value=serde_json::from_str(metadata.settings_json.as_deref().unwrap_or("{}")).map_err(|error|error.to_string())?;
        if settings.get("owner").is_some() || settings.get("conversation").is_some() || settings.get("cliInvocation").is_some() {return Err("unsupported conversation settings fields".into());}
        Ok(serde_json::json!({"conversationId":id,"configurationRevision":metadata.configuration_revision,"contextSource":source,
            "sessionMemory":settings["sessionMemory"].as_str().unwrap_or(""),
            "knowledgePoints":settings["knowledge"]["keyPointsMarkdown"].as_str().unwrap_or(""),
            "wiki":settings["knowledge"]["wikiMarkdown"].as_str().unwrap_or(""),"lastCompactedAt":settings["lastCompactedAt"]}))
    }

    pub fn replace_web_context_summary(&self,source:nola_store_port::context::ContextCompactionSource,summary:String)->Result<bool,String> {
        crate::application_port::CliApplicationPort::persistent(self.path())?.execute(ApplicationCommand::Context {
            command:nola_application::ContextCommand::ReplaceSummary {source,summary,committed_at_ms:chrono::Utc::now().timestamp_millis()}
        })
    }

    pub fn delete_web_conversation(&self, conversation_id: &str) -> Result<bool, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let deleted: bool = canonical.execute(ApplicationCommand::DeleteConversation {
            conversation_id: conversation_id.to_string(),
        })?;
        Ok(deleted)
    }

    pub fn load_web_messages_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<Value>, String> {
        Ok(canonical_web_messages(self, conversation_id)?.unwrap_or_default())
    }

    pub fn load_web_message_window(
        &self,
        conversation_id: &str,
        before_ordinal: Option<i64>,
        limit: usize,
    ) -> Result<WebMessageWindow, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let window = canonical.load_conversation_projection_window_v1(
            conversation_id,
            before_ordinal,
            limit.clamp(1, 200),
            600_000,
        )?;
        let projection = serde_json::to_value(&window.projection)
            .map_err(|error| format!("encode Web conversation projection failed: {error}"))?;
        let messages = web_messages_from_projection_value(&projection, conversation_id)?;
        Ok(WebMessageWindow {
            assistant_journal_heads: window.assistant_journal_heads,
            messages,
            projection,
            projection_revision: window.projection.revision,
            event_sequence: window.event_sequence,
            total_count: usize::try_from(window.total_message_count).unwrap_or(usize::MAX),
            before_ordinal: window.before_ordinal,
            has_more: window.has_more,
        })
    }

    pub fn update_web_message(&self, message: &Value) -> Result<bool, String> {
        let id = required_web_id(message, "messageId", "message")?;
        let conversation_id = required_web_id(message, "conversationId", "conversation")?;
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let Some(conversation) = canonical.load_conversation(&conversation_id)? else {
            return Ok(false);
        };
        let Some(existing) = canonical_web_stored_message(&conversation, &id) else {
            return Ok(false);
        };
        let serialized = serde_json::to_string(message)
            .map_err(|error| format!("serialize web message failed: {error}"))?;
        if existing
            .parts
            .first()
            .and_then(|part| part.content_json.as_deref())
            == Some(serialized.as_str())
        {
            return Ok(true);
        }
        let ledger_count = canonical
            .conversation_summary(&conversation_id)?
            .map(|summary| summary.message_count.max(0) as usize)
            .ok_or_else(|| format!("web conversation {conversation_id} does not exist"))?;
        let replacement_id = format!("web-revision:{id}:{}", conversation.revision + 1);
        let _: Value = canonical.execute(ApplicationCommand::EditConversationMessage {
            input: EditConversationMessage {
                operation_id: format!(
                    "web-update:{conversation_id}:{id}:{}",
                    conversation.revision
                ),
                conversation_id: conversation_id.clone(),
                expected_revision: conversation.revision,
                parent_message_id: existing.id.clone(),
                replacement_message: web_import_message(
                    message,
                    replacement_id,
                    ledger_count as i64 + 1,
                )?,
                committed_at_ms: web_message_time_millis(message),
            },
        })?;
        Ok(true)
    }

    pub fn delete_web_message(
        &self,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<bool, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let Some(conversation) = canonical.load_conversation(conversation_id)? else {
            return Ok(false);
        };
        let Some(existing) = canonical_web_stored_message(&conversation, message_id) else {
            return Ok(false);
        };
        let _: Value = canonical.execute(ApplicationCommand::DeleteConversationMessage {
            input: DeleteConversationMessage {
                operation_id: format!("web-delete:{conversation_id}:{}", existing.id),
                conversation_id: conversation_id.to_string(),
                expected_revision: conversation.revision,
                message_id: existing.id.clone(),
                committed_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })?;
        Ok(true)
    }

    pub fn count_web_user_messages(&self, conversation_id: &str) -> Result<usize, String> {
        Ok(canonical_web_messages(self, conversation_id)?
            .unwrap_or_default()
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .count())
    }

    pub fn create_web_conversation_branch(
        &self,
        source_conversation_id: &str,
        through_message_id: &str,
        conversation: &Value,
    ) -> Result<nola_application::ConversationBranchCreation, String> {
        let selection = web_model_selection(conversation)?;
        let (conversation_id, title, settings_json, created_at_ms, updated_at_ms) =
            canonical_web_profile(conversation, Some(selection.as_deref()))?;
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let window = canonical.load_conversation_projection_window_v1(
            source_conversation_id,
            None,
            1,
            600_000,
        )?;
        let metadata=configuration_metadata(&mut canonical,source_conversation_id)?.ok_or("branch source no longer exists")?;
        if u64::try_from(metadata.revision).ok()!=Some(window.projection.revision) {return Err("branch source changed while reading configuration".into());}
        let mut source_settings:Value=serde_json::from_str(metadata.settings_json.as_deref().unwrap_or("{}")).map_err(|error|error.to_string())?;
        let target_settings:Value=serde_json::from_str(&settings_json).map_err(|error|error.to_string())?;
        source_settings.as_object_mut().ok_or("branch source settings must be an object")?.insert("titleSource".into(),target_settings["titleSource"].clone());
        let settings_json=serde_json::to_string(&source_settings).map_err(|error|error.to_string())?;
        let through_message_id = if through_message_id.is_empty() {
            window
                .projection
                .messages
                .last()
                .map(|message| message.message_id.0.as_str())
                .ok_or("conversation has no complete branch boundary")?
        } else {
            through_message_id
        };
        canonical.create_conversation_branch(CreateConversationBranch {
            operation_id: format!("web-branch:{conversation_id}"),
            source_conversation_id: source_conversation_id.to_string(),
            expected_source_revision: i64::try_from(window.projection.revision)
                .map_err(|_| "conversation revision exceeds the canonical range")?,
            through_message_id: through_message_id.to_string(),
            conversation: CanonicalBranchTarget {
                id: conversation_id,
                title,
                settings_json: Some(settings_json),
                created_at_ms,
                updated_at_ms,
            },
            committed_at_ms: updated_at_ms,
        })
    }

    pub fn load_web_agent_settings(&self) -> Result<Option<Value>, String> {
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let setting: Option<nola_application::ProductSetting> =
            canonical.execute(ApplicationCommand::GetProductSetting {
                key: "web.agent.settings".to_string(),
            })?;
        setting
            .map(|setting| {
                serde_json::from_str::<Value>(&setting.value_json)
                    .map_err(|err| format!("parse web agent settings failed: {err}"))
            })
            .transpose()
    }

    pub fn save_web_agent_settings(&self, value: &Value) -> Result<(), String> {
        let text = serde_json::to_string(value)
            .map_err(|err| format!("serialize web agent settings failed: {err}"))?;
        let mut canonical = crate::application_port::CliApplicationPort::persistent(self.path())?;
        let _: nola_application::ProductSetting =
            canonical.execute(ApplicationCommand::PutProductSetting {
                key: "web.agent.settings".to_string(),
                value_json: text,
                updated_at_ms: web_time_millis(&now_string()),
            })?;
        Ok(())
    }
}

/// Creation is the only implicit-default boundary. Subsequent reads never
/// replace this snapshot with the current model center or selected window.
fn initial_web_configuration(
    store: &NolaStore,
    app: &mut crate::application_port::CliApplicationPort,
    raw: &str,
) -> Result<String, String> {
    use nola_application::model_selection::{ChatModelCandidate, ChatModelIntent, ChatModelResolution, ModelAvailability};
    use serde_json::json;
    let mut settings: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    let mut profiles = store.model_profiles()?;
    profiles.sort_by_key(|profile| !profile.selected);
    let candidates = profiles.iter().map(|profile| {
        let source: Value = serde_json::from_str(&profile.source_json).map_err(|_| "Invalid model source")?;
        Ok(ChatModelCandidate {
            profile_id: profile.id.clone(),
            allowed_for_chat: source["capabilities"].as_array().is_some_and(|values| values.contains(&json!("chat"))),
            availability: if !profile.enabled || profile.deleted_at_ms.is_some() || source["endpointEnabled"] == false || source["modelEnabled"] == false {
                ModelAvailability::Disabled
            } else if profile.runtime != "remote" { ModelAvailability::UnsupportedRuntime } else { ModelAvailability::Available },
        })
    }).collect::<Result<Vec<_>, String>>()?;
    let default: Option<nola_application::ProductSetting> = app.execute(ApplicationCommand::GetProductSetting { key: "chat.default_model".into() })?;
    let selected = settings["modelSelectionID"].as_str().map(str::to_owned);
    let intent = selected.as_deref().and_then(|key| key.strip_prefix("configured:"))
        .map(|id| ChatModelIntent::Explicit { profile_id: id.into() }).unwrap_or(ChatModelIntent::Inherit);
    if !selected.as_deref().is_some_and(|key| key.starts_with("local:")) {
        match nola_application::model_selection::resolve_chat_model(&intent, default.as_ref().map(|value| value.value_json.as_str()), &candidates) {
            ChatModelResolution::Resolved { profile_id, .. } => {
                let key = format!("configured:{profile_id}");
                settings["modelSelectionID"] = json!(key);
                // Store creation materializes model defaults transactionally;
                // adapters only select the identity, never duplicate policy.
            }
            ChatModelResolution::NoAutomaticCandidate => { settings["modelSelectionID"] = Value::Null; }
            _ => return Err("New conversation default model is unavailable; choose an explicit model".into()),
        }
    }
    let global = store.load_web_agent_settings()?.unwrap_or(json!({}));
    let permission = global.get("permission_profile").and_then(Value::as_str).unwrap_or("cautious");
    if crate::permissions::PermissionProfile::from_canonical_name(permission).is_none() { return Err("Invalid default permission profile".into()); }
    settings["permissionModeRawValue"] = json!(permission);
    settings["memoryEnabledOverride"] = json!(crate::assistant_memory_preferences::read_default(app)?);
    settings["retrievalEnabledOverride"] = json!(crate::assistant_retrieval_preferences::read_default(app)?);
    serde_json::to_string(&settings).map_err(|error| error.to_string())
}

#[cfg(test)]
mod creation_configuration_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn creation_materializes_defaults_once_without_copying_credentials() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("fixture.sqlite")).unwrap();
        let mut profile = nola_application::ModelProfile {
            id:"profile".into(), display_name:"Fixture".into(), model_name:"fixture-model".into(),
            provider:"openai-compatible".into(), runtime:"remote".into(),
            source_json:json!({"baseURL":"https://fixture.invalid/v1","endpointID":"endpoint","wireProtocol":"responses",
                "capabilities":["chat"],"executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true},
                "reasoning":{"control":"effort","efforts":["low","high"]},"apiKey":"never-copy-this"}).to_string(),
            parameters_json:json!({"temperature":0.3,"topP":0.8,"maxOutputTokens":8192,"maxContextLength":64000}).to_string(),
            selected:true, enabled:true, revision:1, created_at_ms:1, updated_at_ms:1, deleted_at_ms:None,
        };
        store.upsert_model_profile(profile.clone()).unwrap();
        let view = super::super::WebConversation::new("conversation".into(),"Fixture".into(),"2026-10-15T00:00:00Z".into(),"2026-10-15T00:00:00Z".into());
        store.create_web_conversation(&json!(view)).unwrap();
        let mut app = crate::application_port::CliApplicationPort::persistent(store.path()).unwrap();
        let before = app.load_conversation_settings("conversation").unwrap().unwrap();
        let settings: Value = serde_json::from_str(&before).unwrap();
        assert_eq!(settings["modelSelectionID"], "configured:profile");
        assert_eq!(settings["modelOverrides"]["configured:profile"]["temperature"], 0.3);
        assert_eq!(settings["permissionModeRawValue"], "cautious");
        assert!(!before.contains("never-copy-this"));
        profile.parameters_json = json!({"temperature":1.1}).to_string();
        store.upsert_model_profile(profile).unwrap();
        assert_eq!(app.load_conversation_settings("conversation").unwrap().unwrap(), before);
        let projection = store.get_web_conversation("conversation").unwrap().unwrap();
        assert_eq!(projection["selectedModelId"], "profile");
    }
}

fn web_messages_from_projection_value(
    projection: &Value,
    conversation_id: &str,
) -> Result<Vec<Value>, String> {
    let messages = projection
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "conversation projection has no messages array".to_string())?;
    messages
        .iter()
        .map(|message| {
            let message_id = message
                .get("message_id")
                .and_then(Value::as_str)
                .ok_or_else(|| "projected message has no message_id".to_string())?;
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("projected message {message_id} has no role"))?;
            let parts = message
                .get("parts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let content = parts
                .iter()
                .map(historical_or_plain_display)
                .collect::<Result<Vec<_>, _>>()?
                .join("");
            let created_at_ms = message
                .get("created_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            Ok(serde_json::json!({
                "messageId": message_id,
                "conversationId": conversation_id,
                "role": role,
                "state": message.get("state").cloned().unwrap_or(Value::Null),
                "revision": message.get("revision").cloned().unwrap_or(Value::Null),
                "content": content,
                "parts": parts,
                "includeInContext": true,
                "meta": {
                    "coreMessageId": message_id,
                    "historicalParts": parts.iter().filter(|part| part["kind"] == "historical").collect::<Vec<_>>()
                },
                "createdAt": chrono::DateTime::<chrono::Utc>::from_timestamp_millis(created_at_ms)
                    .map(|date| date.to_rfc3339())
                    .unwrap_or_default()
            }))
        })
        .collect()
}

fn canonical_web_messages(
    store: &NolaStore,
    conversation_id: &str,
) -> Result<Option<Vec<Value>>, String> {
    let Some(conversation) = store.load_canonical_conversation(conversation_id)? else {
        return Ok(None);
    };
    let messages = conversation
        .messages
        .iter()
        .map(|message| {
            let stored = message.parts.iter().find_map(|part| {
                part.content_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str::<Value>(json).ok())
                    .filter(|value| value.get("messageId").is_some())
            });
            let content = stored
                .as_ref()
                .and_then(|value| value.get("content"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    let text = message
                        .parts
                        .iter()
                        .filter(|part| part.kind == "text" || part.kind == "provider_message")
                        .map(|part| part.content_text.as_str())
                        .collect::<String>();
                    (!text.is_empty()).then_some(text)
                })
                .unwrap_or_default();
            let mut value = stored
                .filter(|value| value.get("messageId").is_some())
                .unwrap_or_else(|| serde_json::json!({}));
            let object = value
                .as_object_mut()
                .ok_or_else(|| format!("canonical Web message {} is not an object", message.id))?;
            object
                .entry("messageId".to_string())
                .or_insert_with(|| Value::String(message.id.clone()));
            object.insert(
                "conversationId".to_string(),
                Value::String(conversation_id.to_string()),
            );
            object.insert("role".to_string(), Value::String(message.role.clone()));
            object.insert("content".to_string(), Value::String(content));
            object
                .entry("includeInContext".to_string())
                .or_insert(Value::Bool(true));
            object
                .entry("meta".to_string())
                .or_insert_with(|| serde_json::json!({"coreMessageId": message.id}));
            object.insert(
                "createdAt".to_string(),
                Value::String(
                    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(message.created_at_ms)
                        .map(|date| date.to_rfc3339())
                        .unwrap_or_default(),
                ),
            );
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Some(messages))
}

fn canonical_web_stored_message<'a>(
    conversation: &'a Conversation,
    web_message_id: &str,
) -> Option<&'a nola_application::ConversationMessage> {
    conversation.messages.iter().find(|message| {
        if message.id == web_message_id {
            return true;
        }
        message
            .parts
            .first()
            .and_then(|part| part.content_json.as_deref())
            .and_then(|json| serde_json::from_str::<Value>(json).ok())
            .and_then(|value| {
                value
                    .get("messageId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .as_deref()
            == Some(web_message_id)
    })
}

fn web_message_time_millis(message: &Value) -> i64 {
    non_empty_web_string(message, "createdAt")
        .map(|value| web_time_millis(&value))
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis())
}

fn web_import_message(
    message: &Value,
    ledger_message_id: String,
    ordinal: i64,
) -> Result<MessageInput, String> {
    let content_json = serde_json::to_string(message)
        .map_err(|error| format!("serialize web message failed: {error}"))?;
    let content_text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let created_at_ms = web_message_time_millis(message);
    Ok(MessageInput {
        id: ledger_message_id.clone(),
        run_id: message
            .get("meta")
            .and_then(|meta| meta.get("runId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        role: web_string(message, "role").unwrap_or_else(|| "user".to_string()),
        status: "completed".to_string(),
        ordinal,
        created_at_ms,
        completed_at_ms: Some(created_at_ms),
        parts: vec![MessagePartInput {
            id: format!("{ledger_message_id}:body"),
            // This is semantically a text Part. The serialized Web message is
            // retained as transport metadata, but must not change the product
            // projection type into an opaque structured Part.
            kind: "text".to_string(),
            ordinal: 1,
            state: "completed".to_string(),
            content_text,
            content_json: Some(content_json),
            content_digest: None,
            created_at_ms,
            updated_at_ms: created_at_ms,
        }],
    })
}

#[cfg(test)]
mod large_conversation_tests {
    use super::*;
    use nola_protocol::{ConversationTranscriptImportSource, ImportConversationTranscript};

    #[test]
    fn web_branch_keeps_one_large_message_and_exports_its_original_body() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("branch.db")).unwrap();
        let profile = |id: &str| {
            serde_json::json!(super::super::WebConversation::new(
                id.into(),
                id.into(),
                "2026-09-08T00:00:00Z".into(),
                "2026-09-08T00:00:00Z".into()
            ))
        };
        store.create_web_conversation(&profile("source")).unwrap();
        assert!(store.patch_web_conversation_configuration("source",1,serde_json::json!({"memoryEnabledOverride":false,"permissionProfile":"cautious","modelOverrides":{"local:branch-model":{"temperature":0.4}}})).unwrap().is_committed());
        let original = "x".repeat(10_000_000 - 4) + "TAIL";
        // Fixture admission is in-process; the behavior under test is the Web
        // branch intent and its bounded projection/export consumers.
        store
            .application()
            .unwrap()
            .import_conversation_transcript(ImportConversationTranscript {
                import_id: "fixture".into(),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: "source".into(),
                expected_message_count: 0,
                imported_at_ms: 2,
                messages: vec![MessageInput {
                    id: "message".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![MessagePartInput {
                        id: "part".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: original.clone(),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                }],
            })
            .unwrap();
        let branch = store
            .create_web_conversation_branch("source", "", &profile("branch"))
            .unwrap();
        let mut branch_app=crate::application_port::CliApplicationPort::persistent(store.path()).unwrap();
        let branch_settings:Value=serde_json::from_str(&branch_app.load_conversation_settings("branch").unwrap().unwrap()).unwrap();
        assert_eq!(branch_settings["memoryEnabledOverride"],false);
        assert_eq!(branch_settings["permissionModeRawValue"],"cautious");
        assert_eq!(branch_settings["modelOverrides"]["local:branch-model"]["temperature"],0.4);
        assert_eq!(branch.message_count, 1);
        let window = store.load_web_message_window("branch", None, 10).unwrap();
        assert_eq!(window.total_count, 1);
        assert_eq!(window.messages.len(), 1);
        assert!(window.messages[0]["content"].as_str().unwrap().len() < original.len());
        let exported = store.load_web_messages_for_conversation("branch").unwrap();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0]["content"].as_str().unwrap(), original);
        let second = store
            .create_web_conversation_branch("branch", "", &profile("second"))
            .unwrap();
        assert_eq!(second.message_count, 1);
        let exported = store.load_web_messages_for_conversation("second").unwrap();
        assert_eq!(exported[0]["content"].as_str().unwrap(), original);
        let canonical = store.load_canonical_archive("second").unwrap().unwrap();
        let part = &canonical["messages"][0]["parts"][0];
        assert_eq!(part["content_text"].as_str().unwrap(), original);
        assert!(part["content_json"].is_null());
        let derivation = &part["derivation"];
        assert_eq!(derivation["target"]["owner"]["part_id"], part["id"]);
        assert_eq!(
            derivation["immediate_source"]["owner"]["conversation_id"],
            "branch"
        );
        assert_eq!(
            derivation["root_source"]["owner"]["conversation_id"],
            "source"
        );
        assert!(derivation.get("provenance").is_none());
        let original_archive = store.load_canonical_archive("source").unwrap().unwrap();
        assert!(original_archive["messages"][0]["parts"][0]
            .get("derivation")
            .is_none());
    }

    fn create_large_conversation(store: &NolaStore, id: &str, character_count: usize) {
        store
            .create_web_conversation(&serde_json::json!(super::super::WebConversation::new(
                id.into(),
                format!("Large {id}"),
                "2026-08-27T00:00:00Z".into(),
                "2026-08-27T00:00:00Z".into()
            )))
            .unwrap();
        let message_count = 200usize;
        let characters_per_message = character_count / message_count;
        let mut application =
            crate::application_port::CliApplicationPort::persistent(store.path()).unwrap();
        for batch_start in (0..message_count).step_by(20) {
            let messages = (batch_start..(batch_start + 20).min(message_count))
                .map(|index| {
                    let value = serde_json::json!({
                        "messageId": format!("{id}-message-{index:04}"),
                        "conversationId": id,
                        "role": if index % 2 == 0 { "user" } else { "assistant" },
                        "content": "水".repeat(characters_per_message),
                        "includeInContext": true,
                        "meta": {
                            "toolCalls": if index % 10 == 0 {
                                serde_json::json!([{"name": "read", "arguments": {"path": format!("/tmp/{index}.txt")}}])
                            } else {
                                Value::Null
                            },
                            "skill": if index % 11 == 0 { "document-review" } else { "" }
                        },
                        "createdAt": format!("2026-08-27T00:{:02}:{:02}Z", (index / 60) % 60, index % 60)
                    });
                    web_import_message(&value, format!("{id}-ledger-{index:04}"), index as i64 + 1)
                        .unwrap()
                })
                .collect::<Vec<_>>();
            application
                .import_conversation_transcript(ImportConversationTranscript {
                    import_id: format!("large-web-import-{id}-{batch_start}"),
                    source: ConversationTranscriptImportSource::ExternalArchive,
                    conversation_id: id.to_string(),
                    expected_message_count: batch_start,
                    messages,
                    imported_at_ms: 1_777_244_400_000 + batch_start as i64,
                })
                .unwrap();
        }
    }

    #[test]
    fn two_ten_million_character_web_conversations_restore_through_bounded_windows() {
        let _guard = crate::store::test_env_lock();
        let root = std::env::temp_dir().join(format!(
            "nola-web-large-window-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = NolaStore::open(&root.join("nola.db")).unwrap();
        for id in ["large-web-a", "large-web-b"] {
            create_large_conversation(&store, id, 10_000_000);
        }

        let summaries = store.load_web_conversations().unwrap();
        assert_eq!(summaries.len(), 2);
        for id in ["large-web-a", "large-web-b", "large-web-a"] {
            let first = store.load_web_message_window(id, None, 24).unwrap();
            assert!(!first.messages.is_empty());
            assert!(first.messages.len() <= 24);
            assert!(first.projection["messages"].as_array().unwrap().len() <= 24);
            assert!(serde_json::to_vec(&first.projection).unwrap().len() <= 700_000);
            assert_eq!(first.total_count, 200);
            assert!(first.has_more);
            assert_eq!(
                Some(first.projection_revision),
                first.projection["revision"].as_u64()
            );
            let expected_projection_revision = first.projection_revision;
            let expected_event_sequence = first.event_sequence;
            let first_payload_characters = first
                .messages
                .iter()
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .map(str::chars)
                .map(Iterator::count)
                .sum::<usize>();
            assert!(first_payload_characters <= 600_000);
            assert!(first
                .messages
                .iter()
                .any(|message| message.get("meta").is_some()));

            let mut restored_message_ids = first
                .messages
                .iter()
                .filter_map(|message| message.get("messageId").and_then(Value::as_str))
                .map(str::to_string)
                .collect::<HashSet<_>>();
            let mut before = first.before_ordinal;
            let mut has_more = first.has_more;
            while has_more {
                let window = store.load_web_message_window(id, before, 24).unwrap();
                assert!(window.projection["messages"].as_array().unwrap().len() <= 24);
                assert!(serde_json::to_vec(&window.projection).unwrap().len() <= 700_000);
                assert_eq!(window.projection_revision, expected_projection_revision);
                assert_eq!(window.event_sequence, expected_event_sequence);
                restored_message_ids.extend(
                    window
                        .messages
                        .iter()
                        .filter_map(|message| message.get("messageId").and_then(Value::as_str))
                        .map(str::to_string),
                );
                before = window.before_ordinal;
                has_more = window.has_more;
            }
            assert_eq!(restored_message_ids.len(), 200);
            let archive = store.load_canonical_archive(id).unwrap().unwrap();
            let canonical_characters = archive["messages"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|message| message["parts"].as_array().into_iter().flatten())
                .filter_map(|part| part["content_text"].as_str())
                .map(str::chars)
                .map(Iterator::count)
                .sum::<usize>();
            assert_eq!(canonical_characters, 10_000_000);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn web_time_millis(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.timestamp_millis())
        .unwrap_or_else(|_| chrono::Utc::now().timestamp_millis())
}

fn web_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn non_empty_web_string(value: &Value, key: &str) -> Option<String> {
    web_string(value, key).filter(|item| !item.trim().is_empty())
}

fn required_web_id(value: &Value, key: &str, label: &str) -> Result<String, String> {
    let id = web_string(value, key)
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .ok_or_else(|| format!("{label} id is required"))?;
    if id.chars().count() > 256 {
        return Err(format!("{label} id exceeds 256 characters"));
    }
    Ok(id)
}

fn validate_web_message_batch(
    messages: &[Value],
    expected_conversation_id: Option<&str>,
) -> Result<(), String> {
    if messages.len() > 10_000 {
        return Err("web message replacement is limited to 10000 messages".to_string());
    }
    let mut ids = HashSet::new();
    for (index, message) in messages.iter().enumerate() {
        let id = required_web_id(message, "messageId", "message")?;
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate web message id: {id}"));
        }
        if let Some(expected) = expected_conversation_id {
            if let Some(actual) = non_empty_web_string(message, "conversationId") {
                if actual.trim() != expected {
                    return Err(format!(
                        "web message at index {index} belongs to conversation {}, expected {expected}",
                        actual.trim()
                    ));
                }
            }
        }
        let encoded = serde_json::to_vec(message)
            .map_err(|err| format!("serialize web message at index {index} failed: {err}"))?;
        if encoded.len() > 1024 * 1024 {
            return Err(format!(
                "web message at index {index} exceeds the 1 MiB storage limit"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{required_web_id, validate_web_message_batch, NolaStore};
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn historical_budget_state_and_opaque_text_remain_visible_without_tools() {
        let projection: serde_json::Value = serde_json::from_str(include_str!(
            "../../../core/tests/fixtures/historical_unloaded_conversation_projection_v1.json"
        ))
        .unwrap();
        let messages = super::web_messages_from_projection_value(&projection, "history").unwrap();
        assert_eq!(messages[0]["parts"], projection["messages"][0]["parts"]);
        let content = messages[0]["content"].as_str().unwrap();
        assert_eq!(content, "opaque original evidence");
        assert!(!content.contains("not_loaded"));
        assert!(!content.contains("selection_budget"));
        assert!(!content.contains("must not run"));
    }

    #[test]
    fn web_history_preserves_historical_parts_without_live_tool_mapping() {
        let projection: serde_json::Value = serde_json::from_str(include_str!(
            "../../../core/tests/fixtures/historical_conversation_projection_v1.json"
        ))
        .unwrap();
        let messages = super::web_messages_from_projection_value(&projection, "history").unwrap();
        assert_eq!(messages[0]["parts"], projection["messages"][0]["parts"]);
        let content = messages[0]["content"].as_str().unwrap();
        assert!(content.contains("  原文\\n🙂  "));
        assert!(!content.contains("supported"));
        assert!(!content.contains("deferred"));
        assert!(!content.contains("unknown_kind"));
        assert!(!content.contains("must not run"));
    }

    #[test]
    fn web_storage_rejects_removed_conversation_id_alias() {
        let value = json!({"conversation_id": "conversation-old"});
        assert!(required_web_id(&value, "conversationId", "conversation").is_err());
    }

    #[test]
    fn web_storage_rejects_removed_message_field_aliases() {
        let messages = vec![json!({
            "message_id": "message-old",
            "conversation_id": "conversation-old",
            "created_at": "2026-08-21T00:00:00Z",
            "role": "user",
            "content": "Old shape"
        })];
        assert!(validate_web_message_batch(&messages, Some("conversation-old")).is_err());
    }

    #[test]
    fn web_context_usage_is_projected_from_application_not_conversation_settings() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nola-web-context-projection-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let store = NolaStore::open(&root.join("nola.db")).unwrap();
        let mut profile = super::super::WebConversation::new(
            "conversation-context".into(),
            "Canonical context".into(),
            "2026-08-24T00:00:00Z".into(),
            "2026-08-24T00:00:00Z".into(),
        );
        profile.last_context_usage = Some(json!({"used_tokens":999_999}));
        store.create_web_conversation(&json!(profile)).unwrap();

        let loaded = store
            .get_web_conversation("conversation-context")
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.get("lastContextUsage"),
            Some(&serde_json::Value::Null)
        );

        drop(store);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn web_context_usage_keeps_known_zero_separate_from_absence() {
        let mut profile = json!({"lastContextUsage": {"used_tokens": 999_999}});
        super::project_application_conversation_state(&mut profile, None, 0);
        assert!(profile["lastContextUsage"].is_null());
        super::project_application_conversation_state(
            &mut profile,
            Some(&json!({"used_tokens": 0, "context_window_tokens": 0})),
            0,
        );
        assert!(profile["lastContextUsage"].is_null());
        let known = json!({"used_tokens": 0, "context_window_tokens": 65536,
            "prompt_token_limit": 49152, "estimated": false, "measured_at_revision": 3});
        super::project_application_conversation_state(&mut profile, Some(&known), 0);
        assert_eq!(profile["lastContextUsage"], known);
    }
}
