// SPDX-License-Identifier: Apache-2.0
//! Complete Web views assembled from canonical identity and owned metadata.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary()->nola_application::ConversationSummary {
        nola_application::ConversationSummary {id:"c".into(),title:"Canonical title".into(),revision:7,message_count:0,
            created_at_ms:1000,updated_at_ms:2000,project_root:None,first_user_message:None,last_user_message_at_ms:None}
    }

    #[test]
    fn canonical_projection_has_no_client_owned_configuration_copy() {
        let settings=json!({"modelSelectionID":"configured:model","executionBackend":"codex","externalExecutionSessions":{"codex":"session"},
            "isPinned":true,"sessionMemory":"summary","knowledge":{"keyPointsMarkdown":"facts","wikiMarkdown":"# Wiki","updatedAt":null},"titleSource":"user"});
        let view=assemble(&summary(),Some(&settings.to_string()),4).unwrap();
        assert_eq!(view.configuration_revision,4); assert_eq!(view.revision,7);
        assert_eq!(view.title,"Canonical title"); assert_eq!(view.selected_model_id,"model");
        assert_eq!(view.execution_backend,"codex"); assert_eq!(view.session_memory,"summary");
        assert_eq!(view.knowledge_points,"facts"); assert!(view.is_pinned);
        assert_ne!(view.created_at,"old");
        for raw in [r#"{"owner":"web","conversation":{}}"#,r#"{"cliInvocation":{}}"#] {
            assert!(assemble(&summary(),Some(raw),1).is_err(),"only the current configuration format is supported");
        }
    }

    #[test]
    fn execution_host_binding_does_not_reactivate_a_model_identity() {
        let raw=json!({"modelSelectionID":"configured:stale","executionHostBinding":{"baseURL":"https://host.example"}});
        let view=assemble(&summary(),Some(&raw.to_string()),2).unwrap();
        assert_eq!(view.selected_model_source,"auto"); assert_eq!(view.selected_model_id,"");
    }

    #[test]
    fn creation_sparse_configuration_cas_and_title_source_use_canonical_commands() {
        let directory=tempfile::tempdir().unwrap();
        let database=directory.path().join("web.db");
        let store=super::super::NolaStore::open(&database).unwrap();
        let view=WebConversation::new("c".into(),"New conversation".into(),"2026-10-15T00:00:00Z".into(),"2026-10-15T00:00:00Z".into());
        store.create_web_conversation(&json!(view)).unwrap();
        let mut app=crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let initial:Value=serde_json::from_str(&app.load_conversation_settings("c").unwrap().unwrap()).unwrap();
        assert!(initial.get("owner").is_none()); assert!(initial.get("conversation").is_none());
        let first=store.patch_web_conversation_configuration("c",1,json!({"executionBackend":"codex"})).unwrap();
        assert!(first.is_committed());
        let stale=store.patch_web_conversation_configuration("c",1,json!({"permissionProfile":"allow_all"})).unwrap();
        assert!(stale.is_conflict());
        let current=store.get_web_conversation("c").unwrap().unwrap();
        let revision=current["revision"].as_i64().unwrap();
        assert!(store.rename_web_conversation("c","User title",ConversationTitleSource::User,revision).unwrap());
        let saved=store.get_web_conversation("c").unwrap().unwrap();
        assert_eq!(saved["title"],"User title"); assert_eq!(saved["titleSource"],"user");
        assert_eq!(saved["executionBackend"],"codex"); assert_eq!(saved["configurationRevision"],2);
        assert!(!store.web_automatic_title_is_current("c").unwrap());
        assert!(store.patch_web_conversation_configuration("c",2,json!({"memoryEnabledOverride":false})).unwrap().is_committed());
        assert_eq!(store.get_web_conversation("c").unwrap().unwrap()["title"],"User title");
        assert!(store.rename_web_conversation("c","Stale title",ConversationTitleSource::User,revision).is_err());
        assert_eq!(store.get_web_conversation("c").unwrap().unwrap()["title"],"User title");
    }

    #[test]
    fn cross_client_reads_do_not_write_defaults() {
        let directory=tempfile::tempdir().unwrap();
        let database=directory.path().join("native.db");
        let mut app=crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        app.create_conversation(nola_application::CreateConversation {id:"c".into(),title:"Native title".into(),settings_json:Some(json!({"executionBackend":"nola","permissionModeRawValue":"cautious"}).to_string()),created_at_ms:1,updated_at_ms:1}).unwrap();
        let before=app.load_conversation_settings("c").unwrap();
        let store=super::super::NolaStore::open(&database).unwrap();
        let one=store.get_web_conversation("c").unwrap().unwrap();
        assert_eq!(store.load_web_conversations().unwrap(),vec![one]);
        assert_eq!(app.load_conversation_settings("c").unwrap(),before);
    }

    #[test]
    fn agent_contexts_are_hidden_from_chat_inventory_but_keep_their_direct_room_link() {
        let directory=tempfile::tempdir().unwrap();
        let database=directory.path().join("contexts.db");
        let mut app=crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        let room_id="00000000-0000-4000-8000-000000000001";
        let context_id="00000000-0000-4000-8000-000000000002";
        let ordinary_id="00000000-0000-4000-8000-000000000003";
        app.create_conversation(nola_application::CreateConversation { id:context_id.into(), title:"Researcher".into(),
            settings_json:Some(json!({"agentRoomOwner":{"room_id":room_id,"participant_id":"participant-1","agent_id":"agent-1"}}).to_string()),
            created_at_ms:1,updated_at_ms:1 }).unwrap();
        app.create_conversation(nola_application::CreateConversation { id:ordinary_id.into(), title:"Planning".into(),
            settings_json:Some(json!({}).to_string()), created_at_ms:2,updated_at_ms:2 }).unwrap();
        let store=super::super::NolaStore::open(&database).unwrap();
        let inventory=store.load_web_conversations().unwrap();
        assert_eq!(inventory.len(),1);
        assert_eq!(inventory[0]["conversationId"],ordinary_id);
        let direct=store.get_web_conversation(context_id).unwrap().expect("historical Room context remains directly addressable");
        assert_eq!(direct["conversationId"],context_id);
        assert!(direct.get("agentRoomOwner").is_none());
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WebConversation {
    pub conversation_id: String,
    pub configuration_revision: i64,
    pub revision: i64,
    pub title: String,
    // This is projected from the shared top-level settings, never the Web profile.
    #[serde(default)]
    pub is_pinned: bool,
    pub title_source: ConversationTitleSource,
    pub session_memory: String,
    pub knowledge_points: String,
    pub wiki: String,
    pub last_compacted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_context_usage: Option<Value>,
    pub selected_model_source: String,
    pub selected_model_id: String,
    pub execution_backend: String,
    pub external_execution_sessions: HashMap<String, String>,
    pub created_at: String,
    pub updated_at: String,
}

pub(crate) use nola_domain::conversation_configuration::ConversationTitleSource;

/// Internal agent contexts remain addressable through the historical Room
/// UI, but are not standalone entries in the ordinary conversation list.
pub(crate) fn is_agent_context(settings_json: Option<&str>) -> Result<bool, String> {
    let Some(raw) = settings_json else { return Ok(false); };
    let settings: Value = serde_json::from_str(raw).map_err(|error| error.to_string())?;
    let Some(owner) = settings.get(nola_domain::agent_room::CONTEXT_OWNER_FIELD) else { return Ok(false); };
    serde_json::from_value::<nola_domain::agent_room::ContextOwner>(owner.clone())
        .map_err(|error| format!("invalid agent context owner: {error}"))?;
    Ok(true)
}

impl WebConversation {
    pub(crate) fn new(id: String, title: String, created_at: String, updated_at: String) -> Self {
        Self {
            conversation_id: id,
            configuration_revision: 1,
            revision: 1,
            title,
            is_pinned: false,
            title_source: ConversationTitleSource::Automatic,
            session_memory: String::new(),
            knowledge_points: String::new(),
            wiki: String::new(),
            last_compacted_at: None,
            last_context_usage: None,
            selected_model_source: "auto".into(),
            selected_model_id: String::new(),
            execution_backend: "nola".into(),
            external_execution_sessions: HashMap::new(),
            created_at,
            updated_at,
        }
    }
}

/// Build a read view only from current canonical settings.
pub(crate) fn assemble(
    summary: &nola_application::ConversationSummary,
    settings: Option<&str>,
    configuration_revision: i64,
) -> Result<WebConversation, String> {
    let mut settings:Value=serde_json::from_str(settings.unwrap_or("{}")).map_err(|error|error.to_string())?;
    let fields=settings.as_object_mut().ok_or("conversation settings must be an object")?;
    if let Some(owner) = fields.remove(nola_domain::agent_room::CONTEXT_OWNER_FIELD) {
        serde_json::from_value::<nola_domain::agent_room::ContextOwner>(owner)
            .map_err(|error| format!("invalid agent context owner: {error}"))?;
    }
    if fields.contains_key("owner") || fields.contains_key("conversation") || fields.contains_key("cliInvocation") {
        return Err("Unsupported conversation settings fields".into());
    }
    let timestamp=|millis| chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)
        .map(|date|date.to_rfc3339()).ok_or("conversation timestamp is outside supported range");
    let mut view=WebConversation::new(summary.id.clone(),summary.title.clone(),timestamp(summary.created_at_ms)?,timestamp(summary.updated_at_ms)?);
    view.configuration_revision=configuration_revision;
    view.revision=summary.revision;
    view.title_source=match fields.get("titleSource") {
        Some(value)=>serde_json::from_value(value.clone()).map_err(|error|format!("invalid title source: {error}"))?,
        None=>ConversationTitleSource::User,
    };
    view.is_pinned=match fields.get("isPinned") { None=>false,Some(Value::Bool(value))=>*value,Some(_)=>return Err("invalid isPinned".into()) };
    for (key,target) in [("sessionMemory",&mut view.session_memory),("executionBackend",&mut view.execution_backend)] {
        if let Some(value)=fields.get(key) { *target=value.as_str().ok_or_else(||format!("invalid {key}"))?.into(); }
    }
    if let Some(knowledge)=fields.get("knowledge").filter(|value|!value.is_null()) {
        view.knowledge_points=knowledge.get("keyPointsMarkdown").and_then(Value::as_str).ok_or("invalid knowledge document")?.into();
        view.wiki=knowledge.get("wikiMarkdown").and_then(Value::as_str).ok_or("invalid knowledge document")?.into();
    }
    view.last_compacted_at=fields.get("lastCompactedAt").and_then(Value::as_str).map(str::to_owned);
    if let Some(sessions)=fields.get("externalExecutionSessions") {
        view.external_execution_sessions=serde_json::from_value(sessions.clone()).map_err(|error|format!("invalid execution sessions: {error}"))?;
    }
    let selection=if fields.get("executionHostBinding").is_some_and(|value|!value.is_null()) { None } else { fields.get("modelSelectionID").filter(|value|!value.is_null()) };
    if let Some(selection)=selection {
        let key=selection.as_str().filter(|key|nola_domain::configuration_patch::valid_model_identity(key)).ok_or("invalid canonical model selection")?;
        let (source,id)=if let Some(id)=key.strip_prefix("configured:") { ("cloud",id) }
            else if let Some(id)=key.strip_prefix("local:") { ("local",id) }
            else { ("connected_host",key) };
        view.selected_model_source=source.into(); view.selected_model_id=id.into();
    }
    Ok(view)
}
