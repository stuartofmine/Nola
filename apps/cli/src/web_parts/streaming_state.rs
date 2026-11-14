fn agent_context_messages_from_request(body: &Value, settings: &AgentSettings) -> Vec<Value> {
    let _ = settings;
    if let Some(conversation_id) = trimmed_string_field(body, "conversationId") {
        if let Ok(store) = NolaStore::open_default() {
            if let Ok((_, messages)) = load_store_conversation_messages(&store, &conversation_id) {
                let mut stored = messages
                    .into_iter()
                    .filter(|message| message.include_in_context)
                    .map(|message| {
                        let mut value = json!({
                            "id": message.message_id,
                            "role": message.role,
                            "content": message.content,
                        });
                        if let Some(map) = message.meta.as_object() {
                            if let Some(core_message_id) = map
                                .get("coreMessageId")
                                .or_else(|| map.get("core_message_id"))
                                .and_then(Value::as_str)
                                .filter(|value| !value.trim().is_empty())
                                .or_else(|| {
                                    map.get("agentTrace")
                                        .and_then(Value::as_object)
                                        .and_then(|trace| trace.get("messageId"))
                                        .and_then(Value::as_str)
                                        .filter(|value| !value.trim().is_empty())
                                })
                            {
                                value["core_message_id"] =
                                    Value::String(core_message_id.to_string());
                            }
                            if let Some(tool_calls) =
                                map.get("tool_calls").or_else(|| map.get("toolCalls"))
                            {
                                value["tool_calls"] = tool_calls.clone();
                            }
                            if let Some(tool_call_id) =
                                map.get("tool_call_id").or_else(|| map.get("toolCallId"))
                            {
                                value["tool_call_id"] = tool_call_id.clone();
                            }
                        }
                        value
                    })
                    .collect::<Vec<_>>();
                let pending_user = body
                    .get("messages")
                    .and_then(Value::as_array)
                    .and_then(|messages| messages.last())
                    .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
                    .filter(|message| {
                        message
                            .get("content")
                            .and_then(Value::as_str)
                            .is_some_and(|content| !content.trim().is_empty())
                    });
                if let Some(pending_user) = pending_user {
                    let already_stored = stored.last().is_some_and(|message| {
                        message.get("role").and_then(Value::as_str) == Some("user")
                            && message.get("content").and_then(Value::as_str)
                                == pending_user.get("content").and_then(Value::as_str)
                    });
                    if !already_stored {
                        stored.push(pending_user.clone());
                    }
                }
                return stored;
            }
        }
    }
    body.get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|message| {
            message
                .get("includeInContext")
                .or_else(|| message.get("include_in_context"))
                .and_then(Value::as_bool)
                .unwrap_or(true)
        })
        .collect()
}

fn persisted_data_source_components_from_request(
    body: &Value,
    settings: &AgentSettings,
    retrieval_enabled: bool,
) -> Result<Vec<ContextComponent>, String> {
    let Some(conversation_id) = trimmed_string_field(body, "conversationId") else {
        return Ok(Vec::new());
    };
    let report_only = body
        .get("contextReportOnly")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    with_data_source_store(|store| persisted_data_source_components_from_store(store, &conversation_id, settings, report_only, retrieval_enabled))
}

fn persisted_data_source_components_from_store(
    store: &mut crate::data_sources::DataSourceStore,
    conversation_id: &str,
    settings: &AgentSettings,
    report_only: bool,
    retrieval_enabled: bool,
) -> Result<Vec<ContextComponent>, String> {
        let source_ids = store
            .sources()
            .iter()
            .map(|source| source.id.clone())
            .collect::<Vec<_>>();
        let mut components = Vec::new();
        for source_id in source_ids {
            let Some(source) = store.source(&source_id).cloned() else {
                continue;
            };
            // Match Mac's web-source selection gate before any HTTP refresh/read or usage record.
            if !retrieval_enabled && source.kind == crate::data_sources::HostDataSourceKind::Http {
                continue;
            }
            let resolution = store.resolve(&source, &conversation_id);
            if !resolution.enabled || !(resolution.usable || resolution.should_refresh) {
                continue;
            }
            let snapshot = match store.read_context_snapshot(&source.id, &conversation_id) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    return Err(format!(
                        "read data source {} for conversation {} failed: {error}",
                        source.id, conversation_id
                    ))
                }
            };
            components.push(ContextComponent {
                kind: ContextComponentKind::PlatformData,
                id: format!("data-source:{}", source.id),
                content: json!({
                    "sourceId": source.id,
                    "title": source.name,
                    "content": snapshot.content,
                    "observedAt": snapshot.observed_at,
                    "truncated": snapshot.truncated,
                }),
                source: Some(ContextComponentSource {
                    platform: Some(ContextPlatform::Host),
                    uri: Some(source.location.clone()),
                    title: Some(source.name.clone()),
                    mime_type: None,
                    observed_at: Some(snapshot.observed_at.clone()),
                    authorization: Some(snapshot.authorization.clone()),
                    maximum_age_minutes: Some(snapshot.maximum_age_minutes),
                }),
            });
            if !report_only {
                let mut metadata = BTreeMap::new();
                metadata.insert("truncated".to_string(), Value::Bool(snapshot.truncated));
                metadata.insert(
                    "connectorId".to_string(),
                    Value::String(source.validation_contract.connector_id.clone()),
                );
                store.record_usage(
                    &source.id,
                    "web_chat",
                    "conversation_context",
                    Some(&conversation_id),
                    web_settings_disclose_to_cloud(settings),
                    metadata,
                )?;
            }
        }
        Ok(components)
}

fn web_settings_disclose_to_cloud(settings: &AgentSettings) -> bool {
    let endpoint = settings.agent_llm_endpoint.trim().to_ascii_lowercase();
    !(endpoint.starts_with("http://127.0.0.1")
        || endpoint.starts_with("http://localhost")
        || endpoint.starts_with("http://[::1]")
        || endpoint.starts_with("https://127.0.0.1")
        || endpoint.starts_with("https://localhost")
        || endpoint.starts_with("https://[::1]"))
}

#[cfg(test)]
mod uploaded_document_context_access_tests {
    use super::*;

    #[test]
    fn retrieval_off_keeps_attachments_and_platform_context_but_removes_web_context() {
        let body = json!({"context_components":[
            {"kind":"web_retrieval","id":"web","content":"WEB-SECRET"},
            {"kind":"attachment","id":"attachment","content":"ATTACHMENT"},
            {"kind":"platform_data","id":"platform","content":"PLATFORM"}
        ], "webRetrieval":[{"id":"web-value","content":"WEB-VALUE-SECRET"}]});
        let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(Vec::new()));
        for enabled in [false, true] {
            let components = web_context_components_with_retrieval(&body, &AgentSettings::default(), &skills, enabled).unwrap();
            let raw = serde_json::to_string(&components).unwrap();
            assert!(raw.contains("ATTACHMENT") && raw.contains("PLATFORM"));
            assert_eq!(raw.contains("WEB-SECRET"), enabled);
            assert_eq!(raw.contains("WEB-VALUE-SECRET"), enabled);
        }
    }

    #[test]
    fn retrieval_off_skips_http_refresh_and_usage_before_network_access() {
        use std::io::{Read, Write};
        use crate::data_sources::{DataSourceDraft, DataSourceStore, HostDataSourceKind};
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("sources.sqlite");
        let mut app = crate::application_port::CliApplicationPort::persistent(&database).unwrap();
        app.create_conversation(nola_application::CreateConversation { id:"c".into(), title:"Fixture".into(), settings_json:None, created_at_ms:1, updated_at_ms:1 }).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let _ = stream.read(&mut [0u8; 4096]).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture").unwrap();
        });
        let mut store = DataSourceStore::open(database).unwrap();
        store.validate_and_save(&DataSourceDraft { id:Some("http".into()), name:"Fixture".into(), kind:HostDataSourceKind::Http,
            location:format!("http://{address}/data"), access_policy:nola_core::data_source::DataSourceAccessPolicy::DefaultEnabled, maximum_age_minutes:60 }).unwrap();
        server.join().unwrap(); // No server remains: an accidental read must fail, not pass silently.
        let before = store.source("http").unwrap().clone();
        assert!(persisted_data_source_components_from_store(&mut store, "c", &AgentSettings::default(), false, false).unwrap().is_empty());
        assert_eq!(store.source("http"), Some(&before));
        assert!(store.usage_records().is_empty());
        assert!(persisted_data_source_components_from_store(&mut store, "c", &AgentSettings::default(), false, true).is_err());
    }

    #[test]
    fn actual_web_context_excludes_unreadable_disabled_and_corrupt_documents() {
        let owned = crate::resource_access::reconcile_data_sources(&[], &[json!({
            "id":"owned", "name":"Owned document", "enabled":true, "content":"VISIBLE-CONTENT"
        })]).unwrap().remove(0);
        let mut foreign = owned.clone();
        foreign["id"] = json!("foreign");
        foreign["content"] = json!("FOREIGN-SECRET");
        foreign["access_owner"]["id"] = json!("different-owner");
        let mut metadata_only = foreign.clone();
        metadata_only["id"] = json!("metadata-only");
        metadata_only["resource_id"] = json!("metadata-only");
        metadata_only["content"] = json!("METADATA-ONLY-SECRET");
        metadata_only["access_grants"] = json!([{
            "grant_id":"metadata-grant", "resource_kind":"data_source", "resource_id":"metadata-only",
            "principal":owned["access_owner"], "permissions":["read_metadata"], "resource_revision":1
        }]);
        assert!(crate::resource_access::data_source_api_projection(&metadata_only).is_some(), "fixture must actually grant metadata visibility");
        let mut disabled = owned.clone();
        disabled["id"] = json!("disabled");
        disabled["enabled"] = json!(false);
        disabled["content"] = json!("DISABLED-SECRET");
        let mut corrupt = owned.clone();
        corrupt["id"] = json!("corrupt");
        corrupt["access_revision"] = json!(0);
        corrupt["content"] = json!("CORRUPT-SECRET");
        let settings = AgentSettings { uploaded_documents: vec![owned, foreign, disabled, corrupt, metadata_only,
            json!({"id":"unowned","enabled":true,"content":"UNOWNED-SECRET"})], ..AgentSettings::default() };
        let skills = crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(Vec::new()));
        let components = web_context_components_with_retrieval(&json!({}), &settings, &skills, true).unwrap();
        let documents = components.iter().filter(|component| component.kind == ContextComponentKind::Document).collect::<Vec<_>>();
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].id, "owned");
        assert_eq!(documents[0].content["content"], "VISIBLE-CONTENT");
        let serialized = serde_json::to_string(&components).unwrap();
        for forbidden in ["FOREIGN-SECRET", "DISABLED-SECRET", "CORRUPT-SECRET", "UNOWNED-SECRET", "METADATA-ONLY-SECRET", "access_owner", "access_grants"] {
            assert!(!serialized.contains(forbidden), "unexpected disclosure: {forbidden}");
        }
        assert_eq!(settings.uploaded_documents[1]["content"], "FOREIGN-SECRET", "projection must not mutate stored records");
    }
}

/// The native Web Host's actual user-instruction producer, shared by capture
/// and ordinary context assembly. Resource templates are resolved by Core.
fn web_user_instruction_values(body: &Value, settings: &AgentSettings) -> Vec<(&'static str, String)> {
    [
        ("web-response-style", trimmed_string_field(body, "responseStyle")
            .or_else(|| trimmed_string_field(body, "response_style"))
            .or_else(|| non_empty_string(settings.response_style.trim()).map(ToOwned::to_owned))),
        ("web-custom-prompt", non_empty_string(settings.custom_system_prompt.trim()).map(ToOwned::to_owned)),
    ].into_iter().filter_map(|(id, value)| value.filter(|v| !v.trim().is_empty()).map(|value| (id, value))).collect()
}

#[cfg(test)]
fn web_context_components_from_request(
    body: &Value,
    settings: &AgentSettings,
) -> Result<Vec<ContextComponent>, String> {
    web_context_components_with_retrieval(
        body,
        settings,
        &crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(None),
        true,
    )
}

fn web_context_components_from_request_with_skills(
    body: &Value,
    settings: &AgentSettings,
    skills: &crate::skill_runtime::SkillRuntimeContext,
) -> Result<Vec<ContextComponent>, String> {
    let mut app = crate::application_port::CliApplicationPort::persistent_default()?;
    let conversation_id = trimmed_string_field(body, "conversationId");
    let enabled = crate::assistant_retrieval_preferences::resolve(&mut app, conversation_id.as_deref(), crate::assistant_retrieval_preferences::requested(body)?)?;
    web_context_components_with_retrieval(body, settings, skills, enabled)
}

fn web_context_components_with_retrieval(
    body: &Value,
    settings: &AgentSettings,
    skills: &crate::skill_runtime::SkillRuntimeContext,
    retrieval_enabled: bool,
) -> Result<Vec<ContextComponent>, String> {
    // Product identity and execution policy are composed exactly once by
    // Core. This boundary carries only typed per-request context; injecting a
    // second platform `system` component duplicated policy and changed model
    // behavior between CLI and Web.
    let mut components = Vec::new();
    if let Some(values) = body
        .get("context_components")
        .or_else(|| body.get("contextComponents"))
        .and_then(Value::as_array)
    {
        components.extend(
            values
                .iter()
                .filter_map(|value| serde_json::from_value::<ContextComponent>(value.clone()).ok())
                // Client context is not the authority for Skill availability.
                .filter(|component| component.kind != ContextComponentKind::Skill)
                .filter(|component| retrieval_enabled || component.kind != ContextComponentKind::WebRetrieval),
        );
    }
    let mut push_text = |kind: ContextComponentKind, id: &str, value: Option<String>| {
        if let Some(content) = value.filter(|value| !value.trim().is_empty()) {
            components.push(ContextComponent {
                kind,
                id: id.to_string(),
                content: Value::String(content),
                source: Some(ContextComponentSource {
                    platform: Some(ContextPlatform::Web),
                    ..ContextComponentSource::default()
                }),
            });
        }
    };
    push_text(
        ContextComponentKind::UserPreference,
        "web-locale",
        Some(
            json!({
                "timezone": trimmed_string_field(body, "timezone"),
                "language": trimmed_string_field(body, "language"),
            })
            .to_string(),
        ),
    );
    for (id, content) in web_user_instruction_values(body, settings) {
        push_text(ContextComponentKind::PromptTemplate, id, Some(content));
    }
    push_text(
        ContextComponentKind::Memory,
        "web-long-term-memory",
        non_empty_string(settings.long_term_memory.trim()).map(ToOwned::to_owned),
    );
    push_text(
        ContextComponentKind::UserPreference,
        "web-user-preferences",
        non_empty_string(settings.user_preferences.trim()).map(ToOwned::to_owned),
    );
    if let Some(conversation_id) = trimmed_string_field(body, "conversationId") {
        if let Ok(store) = NolaStore::open_default() {
            if let Ok((conversation, _)) =
                load_store_conversation_messages(&store, &conversation_id)
            {
                push_text(
                    ContextComponentKind::SessionMemory,
                    "web-session-memory",
                    non_empty_string(conversation.session_memory.trim()).map(ToOwned::to_owned),
                );
            }
        }
    }
    components.extend(persisted_data_source_components_from_request(
        body, settings, retrieval_enabled,
    )?);
    if skills.enabled() {
        // The same owned source produces the prompt used by Core. The index's
        // internal resource paths are not model-visible context or grants.
        components.push(ContextComponent {
            kind: ContextComponentKind::Skill,
            id: "skill-catalog".to_string(),
            content: Value::String(skills.catalog_prompt()?),
            source: Some(ContextComponentSource {
                platform: Some(ContextPlatform::Shared),
                title: Some("Available Skills".to_string()),
                mime_type: Some("text/plain".to_string()),
                ..ContextComponentSource::default()
            }),
        });
    }
    let mut push_values =
        |kind: ContextComponentKind, prefix: &str, values: &[Value], platform: ContextPlatform| {
            for (index, value) in values.iter().enumerate() {
                if value.get("enabled").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                let id = value
                    .get("id")
                    .or_else(|| value.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{prefix}-{index}"));
                components.push(ContextComponent {
                    kind,
                    id,
                    content: value.clone(),
                    source: Some(ContextComponentSource {
                        authorization: None,
                        maximum_age_minutes: None,
                        platform: Some(platform),
                        uri: value
                            .get("uri")
                            .or_else(|| value.get("url"))
                            .or_else(|| value.get("endpoint"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        title: value
                            .get("title")
                            .or_else(|| value.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        mime_type: value
                            .get("mimeType")
                            .or_else(|| value.get("mime_type"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        observed_at: value
                            .get("observedAt")
                            .or_else(|| value.get("observed_at"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    }),
                });
            }
        };
    push_values(
        ContextComponentKind::Document,
        "document",
        &crate::resource_access::data_source_model_context_resources(&settings.uploaded_documents),
        ContextPlatform::Shared,
    );
    push_values(
        ContextComponentKind::Mcp,
        "mcp",
        &crate::resource_access::mcp_model_context_resources(&settings.mcp_servers),
        ContextPlatform::Host,
    );
    if let Some(values) = body.get("attachments").and_then(Value::as_array) {
        push_values(
            ContextComponentKind::Attachment,
            "attachment",
            values,
            ContextPlatform::Web,
        );
    }
    if let Some(values) = body
        .get("webRetrieval")
        .or_else(|| body.get("web_retrieval"))
        .and_then(Value::as_array)
        .filter(|_| retrieval_enabled)
    {
        push_values(
            ContextComponentKind::WebRetrieval,
            "web-retrieval",
            values,
            ContextPlatform::Host,
        );
    }
    if let Some(values) = body
        .get("personalIntelligence")
        .or_else(|| body.get("personal_intelligence"))
        .and_then(Value::as_array)
    {
        push_values(
            ContextComponentKind::PersonalIntelligence,
            "personal-intelligence",
            values,
            ContextPlatform::Shared,
        );
    }
    Ok(components)
}

#[cfg(test)]
fn prepare_web_chat_context(
    messages: &[Value],
    tools: &[Value],
    context_window_tokens: usize,
    reserved_output_tokens: usize,
    keep_recent_messages: usize,
    compacted_tool_chars: usize,
    compacted_assistant_chars: usize,
) -> ContextPrepareOutput {
    prepare_context(&ContextPrepareInput {
        schema_version: CONTEXT_ENGINE_SCHEMA_VERSION,
        mode: ContextPrepareMode::Normal,
        platform: ContextPlatform::Web,
        components: Vec::new(),
        messages: messages.to_vec(),
        tools: tools.to_vec(),
        budget: ContextBudgetSpec {
            context_window_tokens: Some(context_window_tokens),
            prompt_token_limit: None,
            token_threshold: None,
            reserved_output_tokens,
            threshold_percent: 60,
            keep_recent_messages,
            min_messages_before_compaction: 4,
            compacted_tool_chars,
            compacted_assistant_chars,
        },
        existing_summary: None,
        summary_result: None,
        evaluated_at: None,
    })
}

fn split_web_core_items(messages: &[Value]) -> (Vec<AgentItem>, Vec<AgentItem>) {
    let items = messages
        .iter()
        .enumerate()
        .flat_map(|(index, message)| session_message_to_agent_items(message, "web-context", index))
        .collect::<Vec<_>>();
    let current_user_index = items.iter().rposition(|item| {
        matches!(
            item,
            AgentItem::Message {
                role: AgentRole::User,
                ..
            }
        )
    });
    let Some(current_user_index) = current_user_index else {
        return (items, Vec::new());
    };
    let mut initial = items;
    let current = initial.split_off(current_user_index);
    (initial, current)
}

#[allow(dead_code)]
fn positive_usize_field(body: &Value, key: &str) -> Option<usize> {
    let value = body.get(key)?;
    if let Some(value) = value.as_u64().and_then(|value| usize::try_from(value).ok()) {
        return (value > 0).then_some(value);
    }
    value
        .as_str()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn non_negative_int_field(body: &Value, key: &str, default: i64) -> Result<i64, String> {
    let Some(value) = body.get(key) else {
        return Ok(default);
    };
    if value.is_null() || value.as_str() == Some("") {
        return Ok(default);
    }
    let parsed = if let Some(value) = value.as_i64() {
        value
    } else if let Some(value) = value.as_str() {
        value
            .parse::<i64>()
            .map_err(|_| format!("{key} must be an integer"))?
    } else {
        return Err(format!("{key} must be an integer"));
    };
    if parsed < 0 {
        Err(format!("{key} must be non-negative"))
    } else {
        Ok(parsed)
    }
}

fn trimmed_string_field(body: &Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn non_empty_string(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    current: Cursor<Vec<u8>>,
}

impl ChannelReader {
    fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            current: Cursor::new(Vec::new()),
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let current_len = self.current.get_ref().len() as u64;
            if self.current.position() < current_len {
                return self.current.read(out);
            }
            match self.rx.recv() {
                Ok(chunk) => self.current = Cursor::new(chunk),
                Err(_) => return Ok(0),
            }
        }
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(
    request: &mut Request,
) -> Result<T, Response<Box<dyn Read + Send>>> {
    let value = read_json_value(request)?;
    serde_json::from_value(value).map_err(|err| {
        json_response(
            StatusCode(400),
            json!({"error": format!("JSON 解析失败: {err}")}),
        )
    })
}

fn read_json_value(request: &mut Request) -> Result<Value, Response<Box<dyn Read + Send>>> {
    let mut body = String::new();
    request
        .as_reader()
        .take(12 * 1024 * 1024 + 1)
        .read_to_string(&mut body)
        .map_err(|err| {
            json_response(
                StatusCode(400),
                json!({"error": format!("读取请求失败: {err}")}),
            )
        })?;
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&body).map_err(|err| {
        json_response(
            StatusCode(400),
            json!({"error": format!("JSON 解析失败: {err}")}),
        )
    })
}

fn read_mobile_json_value(
    request: &mut Request,
    token: &str,
) -> Result<Value, Response<Box<dyn Read + Send>>> {
    let mut body = String::new();
    request
        .as_reader()
        .read_to_string(&mut body)
        .map_err(|error| {
            json_response(
                StatusCode(400),
                json!({"error": format!("failed to read mobile request: {error}")}),
            )
        })?;
    if body.len() > 12 * 1024 * 1024 {
        return Err(json_response(
            StatusCode(413),
            json!({"error": "mobile request is too large"}),
        ));
    }
    if request_header(request, "X-Nola-Mobile-Auth").is_some() {
        let expected = request_header(request, "X-Nola-Mobile-Body-SHA256").unwrap_or("");
        let actual = nola_core::mobile_crypto::mobile_body_sha256(body.as_bytes());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(json_response(
                StatusCode(401),
                json!({"error": "mobile request body authentication failed"}),
            ));
        }
    }
    let envelope_value = serde_json::from_str::<Value>(&body).map_err(|_| {
        json_response(
            StatusCode(400),
            json!({"error": "invalid encrypted mobile JSON"}),
        )
    })?;
    let envelope =
        serde_json::from_value::<nola_core::mobile_crypto::MobileEncryptedEnvelope>(envelope_value)
            .map_err(|_| {
                json_response(
                    StatusCode(400),
                    json!({"error": "encrypted mobile payload required"}),
                )
            })?;
    nola_core::mobile_crypto::decrypt_mobile_json(&envelope, token)
        .map_err(|error| json_response(StatusCode(400), json!({"error": error})))
}

fn with_state_read<F>(f: F) -> Response<Box<dyn Read + Send>>
where
    F: FnOnce(&WebState) -> Response<Box<dyn Read + Send>>,
{
    match load_state() {
        Ok(state) => f(&state),
        Err(err) => json_response(StatusCode(500), json!({"error": err})),
    }
}

/// Settings reads do not hydrate conversations, message bodies, or task history.
fn load_agent_settings(store: &NolaStore) -> Result<AgentSettings, String> {
    let mut settings = AgentSettings::default();
    if let Some(value) = store.load_web_agent_settings()? {
        if let Ok(saved) = serde_json::from_value::<AgentSettings>(value) {
            settings = saved;
        }
    }
    settings.uploaded_documents = store.load_web_uploaded_documents()?;
    settings.uploaded_skills = crate::skill_catalog_port::load_web_uploaded_skills()?;
    settings.mcp_servers = crate::mcp_catalog_port::load_web_mcp_servers()?;
    Ok(settings)
}

#[cfg(test)]
mod agent_settings_projection_tests {
    use super::*;

    #[test]
    fn settings_only_read_preserves_persisted_projection_and_catalog_authority() {
        let _environment = crate::store::test_env_lock();
        let root = std::env::temp_dir().join(format!("nola-settings-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("nola.db");
        std::env::set_var("NOLA_DB_PATH", &path);
        let store = NolaStore::open(&path).unwrap();
        let mut saved = serde_json::to_value(AgentSettings::default()).unwrap();
        saved["custom_system_prompt"] = json!("Persisted settings projection");
        saved["network_policy"] = json!("deny");
        saved["enable_local_dev_tools"] = json!(true);
        saved["uploaded_documents"] = json!([{"id":"embedded-is-not-catalog-authority"}]);
        store.save_web_agent_settings(&saved).unwrap();
        let actual = load_agent_settings(&store).unwrap();
        assert_eq!(actual.custom_system_prompt, "Persisted settings projection");
        assert_eq!(actual.network_policy, "deny");
        assert!(actual.enable_local_dev_tools);
        assert!(actual.uploaded_documents.is_empty());
        assert_eq!(serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(load_state().unwrap().agent_settings).unwrap());
        std::env::remove_var("NOLA_DB_PATH");
        let _ = std::fs::remove_dir_all(root);
    }
}

fn load_state() -> Result<WebState, String> {
    let mut state = WebState::default();
    let store = NolaStore::open_default()?;
    state.conversations = store
        .load_web_conversations()?
        .into_iter()
        .map(serde_json::from_value::<WebConversation>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("invalid Web conversation projection: {error}"))?;
    state.messages = store
        .load_web_messages()?
        .into_iter()
        .filter_map(|value| serde_json::from_value::<WebMessage>(value).ok())
        .collect();
    state.agent_settings = load_agent_settings(&store)?;
    state.permission_rules = crate::permission_grants::load_web_permission_rules()?
        .into_iter()
        .filter_map(|value| serde_json::from_value::<WebPermissionRule>(value).ok())
        .collect();
    state.scheduled_actions = store
        .load_web_scheduled_actions()?
        .into_iter()
        .filter_map(|value| serde_json::from_value::<WebScheduledAction>(value).ok())
        .collect();
    state.execution_results = store
        .load_web_execution_results()?
        .into_iter()
        .filter_map(|value| serde_json::from_value::<WebExecutionResult>(value).ok())
        .collect();
    Ok(state)
}

fn web_runtime_directory() -> Result<PathBuf, String> {
    let home = crate::platform_paths::home_dir()
        .ok_or_else(|| "cannot resolve the user home directory".to_string())?;
    Ok(home.join(".nola").join("web"))
}

fn chat_conversation_route(path: &str) -> Option<(&str, &str)> {
    let prefix = "/api/chat/conversations/";
    let rest = path.strip_prefix(prefix)?;
    if rest.is_empty() {
        return None;
    }
    let (id, suffix) = rest.split_once('/').unwrap_or((rest, ""));
    if id.is_empty() {
        return None;
    }
    Some((id, suffix))
}

fn data_source_route(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/data-sources/")?;
    if rest.is_empty() || rest == "usage" {
        return None;
    }
    let (id, suffix) = rest.split_once('/').unwrap_or((rest, ""));
    if id.is_empty() {
        return None;
    }
    Some((id, suffix))
}

fn scheduled_action_route(path: &str) -> Option<(&str, &str)> {
    let prefix = "/api/scheduled-actions/";
    let rest = path.strip_prefix(prefix)?;
    if rest.is_empty() {
        return None;
    }
    let (id, suffix) = rest.split_once('/').unwrap_or((rest, ""));
    if id.is_empty() {
        return None;
    }
    Some((id, suffix))
}

fn result_detail_route(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/results/")?;
    if rest.is_empty() || rest.contains('/') {
        None
    } else {
        Some(rest)
    }
}

fn permission_rule_route(path: &str) -> Option<u64> {
    let rest = path.strip_prefix("/api/agent/permission_rules/")?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    rest.parse::<u64>().ok()
}

fn apply_model_selection(
    conversation: &mut WebConversation,
    source: Option<&str>,
    model_id: Option<&str>,
) -> Result<(), String> {
    let source = source
        .unwrap_or(&conversation.selected_model_source)
        .trim()
        .to_ascii_lowercase();
    let mut model_id = model_id
        .unwrap_or(&conversation.selected_model_id)
        .trim()
        .to_string();
    if !matches!(source.as_str(), "auto" | "cloud" | "local") {
        return Err("selectedModelSource must be auto, cloud, or local".to_string());
    }
    if source == "auto" {
        model_id.clear();
    } else if source == "cloud" && model_id.is_empty() {
        return Err("selectedModelId is required for cloud model selection".to_string());
    }
    conversation.selected_model_source = source;
    conversation.selected_model_id = model_id;
    Ok(())
}

fn apply_execution_backend(
    conversation: &mut WebConversation,
    backend: &str,
) -> Result<(), String> {
    let backend = backend.trim();
    let available = crate::external_executor::discover_execution_backends()
        .into_iter()
        .any(|entry| {
            entry.id == backend
                && entry.availability
                    == nola_domain::execution_backend::ExecutionBackendAvailability::Available
        });
    if !available {
        return Err(format!("execution backend is unavailable: {backend}"));
    }
    conversation.execution_backend = backend.to_string();
    Ok(())
}

fn scheduled_action_url(format: &str, input: &ScheduledActionInput) -> Result<String, String> {
    if format == "agent" {
        if input.url.is_some() || input.path.is_some() || input.query.is_some() || input.script.is_some() || input.cwd.is_some() {
            return Err("Agent tasks use executionSpec, not source metadata".into());
        }
        return Ok(String::new());
    }
    let value = match format {
        "search" => input.query.as_deref(),
        "file" | "folder" => input.path.as_deref(),
        "script" => input.cwd.as_deref(),
        _ => input.url.as_deref(),
    }
    .unwrap_or("")
    .trim()
    .to_string();
    if value.is_empty() {
        let msg = match format {
            "search" => "搜索任务缺少关键词",
            "file" => "文件任务缺少路径",
            "folder" => "文件夹任务缺少路径",
            "script" => "脚本任务缺少运行目录",
            _ => "网页任务缺少 URL",
        };
        return Err(msg.to_string());
    }
    if matches!(format, "html" | "rss" | "json" | "icalendar")
        && !(value.starts_with("http://") || value.starts_with("https://"))
    {
        return Err("请输入有效的 http/https URL".to_string());
    }
    Ok(value)
}

fn source_kind_for_format<'a>(format: &'a str, url: &'a str) -> &'a str {
    match format {
        "agent" => "agent",
        "file" => "file",
        "folder" => "folder",
        "script" => "script",
        "search" => "search",
        _ if url.starts_with("file://") => "file",
        _ => "web",
    }
}

fn source_kind_label(kind: &str) -> &str {
    match kind {
        "agent" => "智能体",
        "file" => "文件",
        "folder" => "文件夹",
        "script" => "脚本",
        "search" => "搜索",
        _ => "网页",
    }
}

fn validate_web_schedule(
    frequency: &str,
    time: &str,
    weekday: Option<u8>,
    timezone: &str,
) -> Result<(), &'static str> {
    if !matches!(frequency, "daily" | "weekly") {
        return Err("unsupported schedule frequency");
    }
    if timezone != "local" {
        return Err("only the host local timezone is supported");
    }
    let bytes = time.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes[..2].iter().chain(&bytes[3..]).all(u8::is_ascii_digit)
        || time[..2].parse::<u32>().unwrap_or(24) > 23
        || time[3..].parse::<u32>().unwrap_or(60) > 59
    {
        return Err("time must be HH:MM in the host local timezone");
    }
    if weekday.is_some_and(|day| !(1..=7).contains(&day))
        || (frequency == "weekly" && weekday.is_none())
    {
        return Err("weekday must be 1 (Sunday) through 7 (Saturday)");
    }
    Ok(())
}

fn estimate_next_scheduled_at(
    frequency: &str,
    time: &str,
    weekday: Option<u8>,
    enabled: bool,
) -> Option<String> {
    if !enabled {
        return None;
    }
    let now=chrono::Utc::now();
    let page=nola_application::automation_occurrence::query(&nola_application::automation_occurrence::OccurrenceQuery {
        automation_id:"schedule-preview".into(),timezone:"system".into(),frequency:frequency.into(),time:Some(time.into()),
        weekday:weekday.map(|day|u32::from((day+5)%7+1)),once_at_ms:None,
        after_ms:now.timestamp_millis(),through_ms:(now+Duration::days(8)).timestamp_millis(),limit:1,
    }).ok()?;
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(page.occurrences.first()?.scheduled_for_ms).map(|date|date.to_rfc3339())
}


fn build_web_search_snapshot(
    action: &WebScheduledAction,
) -> (String, String, Vec<Value>, Option<String>) {
    let query = action.url.trim();
    if query.is_empty() {
        return (
            "failed".to_string(),
            String::new(),
            Vec::new(),
            Some("Search query is empty".to_string()),
        );
    }
    let mut arguments = serde_json::Map::new();
    arguments.insert("query".to_string(), Value::String(query.to_string()));
    arguments.insert("limit".to_string(), json!(20));
    arguments.insert("engine".to_string(), Value::String("auto".to_string()));
    arguments.insert("parse_pages".to_string(), Value::Bool(true));
    let outcome = crate::tools::web_search_core::execute(&arguments);
    let event = json!({
        "kind": "web_search",
        "query": query,
        "display": outcome.display,
        "report": outcome.text,
        "error": outcome.error,
    });
    if outcome.error {
        (
            "failed".to_string(),
            String::new(),
            vec![event],
            Some(outcome.text),
        )
    } else {
        ("success".to_string(), outcome.text, vec![event], None)
    }
}

fn build_html_snapshot(
    action: &WebScheduledAction,
) -> (String, String, Vec<Value>, Option<String>) {
    let url = action.url.trim();
    if url.is_empty() {
        return failed_snapshot("网页任务缺少 URL");
    }
    let client = match reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(StdDuration::from_secs(15))
        .user_agent("Nola-CLI-Web/1.0 (+local-first html monitor)")
        .build()
    {
        Ok(client) => client,
        Err(err) => return failed_snapshot(format!("创建网页客户端失败：{err}")),
    };
    let response = match client.get(url).send() {
        Ok(response) => response,
        Err(err) => return failed_snapshot(format!("抓取网页失败：{err}")),
    };
    let status_code = response.status().as_u16();
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .split(';')
        .next()
        .unwrap_or("unknown")
        .trim()
        .to_string();
    if status_code >= 400 {
        return failed_snapshot(format!("网页返回异常状态：HTTP {status_code}"));
    }
    let html = match response.text() {
        Ok(text) => text,
        Err(err) => return failed_snapshot(format!("读取网页响应失败：{err}")),
    };
    let title = html_title(&html).unwrap_or_default();
    let description = html_meta_content(&html, "description")
        .or_else(|| html_meta_content(&html, "og:description"))
        .unwrap_or_default();
    let raw_text = clean_html_text(&html);
    let mut preview = if description.trim().is_empty() {
        truncate_chars(&raw_text, 220)
    } else {
        description.clone()
    };
    if preview.trim().is_empty() && raw_text.to_ascii_lowercase().contains("window.location") {
        preview = "目标网页返回了跳转脚本或访问校验页，当前未抓取到可读正文。".to_string();
    }
    let mut pieces = vec![format!("网页监控已完成：成功抓取「{}」。", action.name)];
    if !title.is_empty() {
        pieces.push(format!("页面标题：{title}。"));
    }
    if !preview.is_empty() {
        pieces.push(format!("内容预览：{preview}"));
    }
    let summary = pieces.join(" ");
    let snapshot = json!({
        "kind": "html_snapshot",
        "title": if title.is_empty() { action.name.clone() } else { title },
        "summary": preview,
        "description": description,
        "url": final_url,
        "contentType": content_type,
        "statusCode": status_code,
    });
    ("success".to_string(), summary, vec![snapshot], None)
}

fn build_local_file_snapshot(
    action: &WebScheduledAction,
    settings: &AgentSettings,
) -> (String, String, Vec<Value>, Option<String>) {
    let path = match resolve_local_readable_path(&action.url, settings, "file") {
        Ok(path) => path,
        Err(err) => return failed_snapshot(err),
    };
    let preview = match read_text_preview(&path, MAX_LOCAL_FILE_BYTES, MAX_LOCAL_FILE_CHARS) {
        Ok(text) => text,
        Err(err) => return failed_snapshot(err),
    };
    let size = path.metadata().ok().map(|meta| meta.len());
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(action.name.as_str());
    let summary = format!(
        "本地文件数据源「{}」读取完成：{}。内容预览：{}",
        action.name,
        name,
        truncate_chars(&preview, 260)
    );
    (
        "success".to_string(),
        summary,
        vec![json!({
            "kind": "local_file_snapshot",
            "title": name,
            "summary": truncate_chars(&preview, 800),
            "path": path.display().to_string(),
            "sourceKind": "file",
            "sizeBytes": size,
        })],
        None,
    )
}

fn build_local_folder_snapshot(
    action: &WebScheduledAction,
    settings: &AgentSettings,
) -> (String, String, Vec<Value>, Option<String>) {
    let folder = match resolve_local_readable_path(&action.url, settings, "folder") {
        Ok(path) => path,
        Err(err) => return failed_snapshot(err),
    };
    let mut events = Vec::new();
    collect_folder_snapshots(&folder, &folder, &mut events);
    if events.is_empty() {
        return (
            "success".to_string(),
            format!(
                "本地文件夹数据源「{}」读取完成，但未发现可读取文本文件。",
                action.name
            ),
            Vec::new(),
            None,
        );
    }
    let names = events
        .iter()
        .take(5)
        .filter_map(|event| event.get("title").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("、");
    (
        "success".to_string(),
        format!(
            "本地文件夹数据源「{}」读取完成：发现 {} 个可读文件。示例：{}",
            action.name,
            events.len(),
            names
        ),
        events,
        None,
    )
}

fn run_script_automation(
    action: &WebScheduledAction,
    settings: &AgentSettings,
) -> (String, String, Vec<Value>, Option<String>) {
    let meta = load_script_metadata(&action.user_prompt);
    if let Err(err) = validate_script_metadata(&meta) {
        return failed_snapshot(err);
    }
    if !script_is_approved(&meta) {
        return failed_snapshot("script has not been approved for scheduled execution");
    }
    if !(settings.enable_local_dev_tools || settings.host_tools_enabled) {
        return failed_snapshot("local developer tools are disabled");
    }
    let cwd = match resolve_allowed_cwd(
        meta.get("cwd").and_then(Value::as_str).unwrap_or(""),
        &settings.allowed_dirs,
    ) {
        Ok(path) => path,
        Err(err) => return failed_snapshot(err),
    };
    let script = meta.get("script").and_then(Value::as_str).unwrap_or("");
    let accepted_lines = match validate_script_lines(script, &settings.bash_prefixes) {
        Ok(lines) => lines,
        Err(err) => return failed_snapshot(err),
    };
    let timeout = meta
        .get("timeoutSec")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_SCRIPT_TIMEOUT_SEC)
        .clamp(1, MAX_SCRIPT_TIMEOUT_SEC);
    let run = run_bash_script(&cwd, &accepted_lines, timeout);
    let event = json!({
        "kind": "script_run",
        "cwd": cwd.display().to_string(),
        "exitCode": run.exit_code,
        "stdout": run.stdout,
        "stderr": run.stderr,
    });
    if run.success {
        (
            "success".to_string(),
            format!("脚本自动化「{}」执行完成。", action.name),
            vec![event],
            None,
        )
    } else {
        let reason = run
            .failure_reason
            .unwrap_or_else(|| "script execution failed".to_string());
        (
            "failed".to_string(),
            String::new(),
            vec![event],
            Some(reason),
        )
    }
}

fn failed_snapshot(message: impl Into<String>) -> (String, String, Vec<Value>, Option<String>) {
    (
        "failed".to_string(),
        String::new(),
        Vec::new(),
        Some(message.into()),
    )
}

fn html_title(html: &str) -> Option<String> {
    Regex::new("(?is)<title[^>]*>(.*?)</title>")
        .ok()?
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|value| clean_html_text(value.as_str()))
        .filter(|value| !value.is_empty())
}

fn html_meta_content(html: &str, key: &str) -> Option<String> {
    let meta_re = Regex::new("(?is)<meta\\b[^>]*>").ok()?;
    let attr_re = Regex::new("(?is)([a-zA-Z:_-]+)\\s*=\\s*(?:\"([^\"]*)\"|'([^']*)')").ok()?;
    for tag in meta_re.find_iter(html) {
        let mut matched = false;
        let mut content = String::new();
        for captures in attr_re.captures_iter(tag.as_str()) {
            let name = captures
                .get(1)
                .map(|value| value.as_str().to_ascii_lowercase())
                .unwrap_or_default();
            let value = captures
                .get(2)
                .or_else(|| captures.get(3))
                .map(|value| value.as_str())
                .unwrap_or("");
            if matches!(name.as_str(), "name" | "property") && value.eq_ignore_ascii_case(key) {
                matched = true;
            }
            if name == "content" {
                content = html_unescape(value);
            }
        }
        if matched && !content.trim().is_empty() {
            return Some(clean_html_text(&content));
        }
    }
    None
}

fn clean_html_text(raw: &str) -> String {
    let without_scripts = Regex::new("(?is)<script\\b.*?</script>")
        .ok()
        .map(|re| re.replace_all(raw, " ").to_string())
        .unwrap_or_else(|| raw.to_string());
    let without_scripts = Regex::new("(?is)<style\\b.*?</style>")
        .ok()
        .map(|re| re.replace_all(&without_scripts, " ").to_string())
        .unwrap_or(without_scripts);
    let no_tags = Regex::new("(?is)<[^>]+>")
        .ok()
        .map(|re| re.replace_all(&without_scripts, " ").to_string())
        .unwrap_or(without_scripts);
    let text = html_unescape(&no_tags);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn resolve_local_readable_path(
    raw_path: &str,
    settings: &AgentSettings,
    expected: &str,
) -> Result<PathBuf, String> {
    let mut text = raw_path.trim().to_string();
    if let Some(value) = text.strip_prefix("file://") {
        text = percent_decode(value);
    }
    if text.is_empty() {
        return Err("本地数据源缺少路径".to_string());
    }
    let requested = expand_home_local(Path::new(&text));
    if !requested.is_absolute() {
        return Err("本地数据源路径必须是绝对路径".to_string());
    }
    if settings.allowed_dirs.is_empty() {
        return Err("请先在 Agent 权限中配置允许目录，再添加本地文件/文件夹数据源".to_string());
    }
    let requested = requested
        .canonicalize()
        .map_err(|_| format!("本地路径不存在：{}", requested.display()))?;
    let mut allowed = false;
    for base in settings
        .allowed_dirs
        .iter()
        .map(|value| expand_home_local(Path::new(value.trim())))
    {
        if base.as_os_str().is_empty() {
            continue;
        }
        let base = base.canonicalize().unwrap_or(base);
        if requested == base || requested.starts_with(&base) {
            allowed = true;
            break;
        }
    }
    if !allowed {
        return Err("本地数据源路径不在允许目录内".to_string());
    }
    if expected == "file" && !requested.is_file() {
        return Err(format!("本地路径不是文件：{}", requested.display()));
    }
    if expected == "folder" && !requested.is_dir() {
        return Err(format!("本地路径不是文件夹：{}", requested.display()));
    }
    Ok(requested)
}

fn expand_home_local(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

fn read_text_preview(path: &Path, max_bytes: u64, max_chars: usize) -> Result<String, String> {
    let file =
        fs::File::open(path).map_err(|err| format!("读取文件失败：{}: {err}", path.display()))?;
    let mut buf = Vec::new();
    file.take(max_bytes)
        .read_to_end(&mut buf)
        .map_err(|err| format!("读取文件失败：{}: {err}", path.display()))?;
    if buf.contains(&0) {
        return Err(format!("文件不是可读取文本：{}", path.display()));
    }
    let text = String::from_utf8_lossy(&buf).trim().to_string();
    if text.is_empty() {
        return Err(format!("文件不是可读取文本：{}", path.display()));
    }
    Ok(truncate_chars(&text, max_chars))
}

fn collect_folder_snapshots(root: &Path, folder: &Path, events: &mut Vec<Value>) {
    if events.len() >= MAX_FOLDER_FILES {
        return;
    }
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if events.len() >= MAX_FOLDER_FILES {
            break;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        if relative
            .components()
            .any(|component| component.as_os_str().to_string_lossy().starts_with('.'))
        {
            continue;
        }
        if path.is_dir() {
            collect_folder_snapshots(root, &path, events);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Ok(preview) = read_text_preview(&path, 32_000, 4_000) else {
            continue;
        };
        let relative_text = relative.display().to_string();
        let size = path.metadata().ok().map(|meta| meta.len());
        events.push(json!({
            "kind": "local_file_snapshot",
            "title": relative_text,
            "summary": truncate_chars(&preview, 800),
            "path": path.display().to_string(),
            "relativePath": relative.display().to_string(),
            "sourceKind": "folder",
            "sizeBytes": size,
        }));
    }
}

fn build_script_metadata(
    script: &str,
    cwd: &str,
    user_prompt: &str,
    approved: bool,
) -> Result<String, String> {
    if script.trim().is_empty() {
        return Err("script is required".to_string());
    }
    if script.chars().count() > MAX_SCRIPT_CHARS {
        return Err(format!("script is too long; max {MAX_SCRIPT_CHARS} chars"));
    }
    if script.contains('\0') {
        return Err("script contains NUL bytes".to_string());
    }
    Ok(json!({
        "kind": "script_automation",
        "runtime": "bash",
        "script": script,
        "cwd": cwd,
        "origin": "user",
        "approvedAt": approved.then(now_string),
        "timeoutSec": DEFAULT_SCRIPT_TIMEOUT_SEC,
        "userPrompt": user_prompt,
    })
    .to_string())
}

fn load_script_metadata(raw: &str) -> Value {
    let value = serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({}));
    if value.get("kind").and_then(Value::as_str) == Some("script_automation") {
        value
    } else {
        json!({
            "kind": "script_automation",
            "runtime": "bash",
            "script": raw,
            "cwd": "",
            "origin": "user",
            "approvedAt": null,
            "timeoutSec": DEFAULT_SCRIPT_TIMEOUT_SEC,
            "userPrompt": "",
        })
    }
}

fn approve_script_metadata(raw: &str) -> Result<String, String> {
    let mut meta = load_script_metadata(raw);
    validate_script_metadata(&meta)?;
    meta["approvedAt"] = Value::String(now_string());
    Ok(meta.to_string())
}

fn validate_script_metadata(meta: &Value) -> Result<(), String> {
    if meta
        .get("runtime")
        .and_then(Value::as_str)
        .unwrap_or("bash")
        != "bash"
    {
        return Err("only bash script automation is supported".to_string());
    }
    let script = meta.get("script").and_then(Value::as_str).unwrap_or("");
    if script.trim().is_empty() {
        return Err("script is required".to_string());
    }
    if script.chars().count() > MAX_SCRIPT_CHARS {
        return Err(format!("script is too long; max {MAX_SCRIPT_CHARS} chars"));
    }
    if script.contains('\0') {
        return Err("script contains NUL bytes".to_string());
    }
    Ok(())
}

fn script_is_approved(meta: &Value) -> bool {
    meta.get("approvedAt")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|value| !value.is_empty())
        .unwrap_or(false)
}

fn script_preview(meta: &Value) -> String {
    let text = meta
        .get("script")
        .and_then(Value::as_str)
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join(" / ");
    truncate_chars(&text, 160)
}

fn resolve_allowed_cwd(cwd: &str, allowed_dirs: &[String]) -> Result<PathBuf, String> {
    if allowed_dirs.is_empty() {
        return Err("no allowed directories configured".to_string());
    }
    let base_dirs = allowed_dirs
        .iter()
        .map(|value| expand_home_local(Path::new(value.trim())))
        .filter(|path| !path.as_os_str().is_empty())
        .collect::<Vec<_>>();
    let requested = if cwd.trim().is_empty() {
        base_dirs
            .first()
            .cloned()
            .ok_or_else(|| "no allowed directories configured".to_string())?
    } else {
        expand_home_local(Path::new(cwd.trim()))
    };
    let requested = requested
        .canonicalize()
        .map_err(|_| format!("working directory does not exist: {}", requested.display()))?;
    if !requested.is_dir() {
        return Err(format!(
            "working directory is not a directory: {}",
            requested.display()
        ));
    }
    for base in base_dirs {
        let base = base.canonicalize().unwrap_or(base);
        if requested == base || requested.starts_with(&base) {
            return Ok(requested);
        }
    }
    Err("working directory is outside allowed directories".to_string())
}

fn validate_script_lines(script: &str, bash_prefixes: &[String]) -> Result<Vec<String>, String> {
    let prefixes = bash_prefixes
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if prefixes.is_empty() {
        return Err("no bash prefixes whitelisted".to_string());
    }
    let forbidden_tokens = [";", "&&", "||", "|", "`", "$(", "${", "<", ">", "\r"];
    let forbidden_commands = [
        ".", "source", "eval", "exec", "sudo", "su", "ssh", "scp", "sftp", "curl", "wget", "nc",
        "ncat", "telnet", "rm",
    ];
    let mut accepted = Vec::new();
    for (index, raw_line) in script.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if forbidden_tokens.iter().any(|token| line.contains(token)) {
            return Err(format!(
                "line {}: shell control operators/redirection are not allowed",
                index + 1
            ));
        }
        let head = command_head(line);
        if head.is_empty() {
            continue;
        }
        if forbidden_commands.contains(&head.as_str()) {
            return Err(format!(
                "line {}: command '{}' is not allowed in scheduled scripts",
                index + 1,
                head
            ));
        }
        if !prefixes
            .iter()
            .any(|prefix| line.starts_with(*prefix) || head == *prefix)
        {
            return Err(format!(
                "line {}: command '{}' is not in the bash prefix whitelist",
                index + 1,
                head
            ));
        }
        accepted.push(line.to_string());
    }
    if accepted.is_empty() {
        return Err("script has no executable command lines".to_string());
    }
    Ok(accepted)
}

fn command_head(line: &str) -> String {
    for part in line.split_whitespace() {
        if part.contains('=')
            && part
                .chars()
                .next()
                .map(|c| c.is_ascii_alphabetic())
                .unwrap_or(false)
        {
            continue;
        }
        return part.trim_matches(|ch| ch == '\'' || ch == '"').to_string();
    }
    String::new()
}

#[derive(Debug)]
struct ScriptRun {
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    failure_reason: Option<String>,
}

#[cfg(unix)]
fn run_bash_script(cwd: &Path, lines: &[String], timeout_sec: u64) -> ScriptRun {
    let sandbox = std::env::temp_dir().join(format!("nola-script-{}", new_id("tmp")));
    if let Err(err) = fs::create_dir_all(&sandbox) {
        return ScriptRun {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            failure_reason: Some(format!("failed to create script sandbox: {err}")),
        };
    }
    let script_path = sandbox.join("script.sh");
    let script = format!("set -euo pipefail\n{}\n", lines.join("\n"));
    if let Err(err) = fs::write(&script_path, script) {
        let _ = fs::remove_dir_all(&sandbox);
        return ScriptRun {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            failure_reason: Some(format!("failed to write script: {err}")),
        };
    }
    let mut child = match Command::new("/bin/bash")
        .arg("--noprofile")
        .arg("--norc")
        .arg(&script_path)
        .current_dir(cwd)
        .env_clear()
        .env("HOME", &sandbox)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("NOLA_SCRIPT_AUTOMATION", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_dir_all(&sandbox);
            return ScriptRun {
                success: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                failure_reason: Some(format!("failed to start script: {err}")),
            };
        }
    };
    let deadline = Instant::now() + StdDuration::from_secs(timeout_sec);
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                break;
            }
            Ok(None) => thread::sleep(StdDuration::from_millis(50)),
            Err(_) => break,
        }
    }
    let output = child.wait_with_output();
    let _ = fs::remove_dir_all(&sandbox);
    match output {
        Ok(output) => {
            let stdout = truncate_chars(
                &String::from_utf8_lossy(&output.stdout),
                MAX_SCRIPT_LOG_CHARS,
            );
            let stderr = truncate_chars(
                &String::from_utf8_lossy(&output.stderr),
                MAX_SCRIPT_LOG_CHARS,
            );
            let exit_code = output.status.code();
            let success = output.status.success() && !timed_out;
            let failure_reason = if timed_out {
                Some(format!("script timed out after {timeout_sec}s"))
            } else if success {
                None
            } else {
                Some(format!(
                    "script exited with code {}",
                    exit_code.unwrap_or(-1)
                ))
            };
            ScriptRun {
                success,
                exit_code,
                stdout,
                stderr,
                failure_reason,
            }
        }
        Err(err) => ScriptRun {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            failure_reason: Some(format!("failed to collect script output: {err}")),
        },
    }
}

#[cfg(not(unix))]
fn run_bash_script(_cwd: &Path, _lines: &[String], _timeout_sec: u64) -> ScriptRun {
    ScriptRun {
        success: false,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        failure_reason: Some(
            "Bash script automation is unavailable on this platform; use native Nola tools or run this automation on a Unix Host."
                .to_string(),
        ),
    }
}

fn latest_result_for_action_values(
    results: &[WebExecutionResult],
    action_id: &str,
) -> Option<Value> {
    results
        .iter()
        .filter(|result| result.source_id == action_id)
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .map(result_latest_value)
}

fn result_latest_value(result: &WebExecutionResult) -> Value {
    json!({
        "resultId": result.result_id,
        "status": result.status,
        "createdAt": result.created_at,
    })
}

fn result_summary_value(result: &WebExecutionResult) -> Value {
    json!({
        "conversationId":result.conversation_id,
        "runId":result.run_id,
        "resultId": result.result_id,
        "jobId": result.job_id,
        "sourceId": result.source_id,
        "sourceName": result.source_name,
        "sourceUrl": result.source_url,
        "sourceFormat": result.source_format,
        "status": result.status,
        "scheduledAt": result.scheduled_at,
        "createdAt": result.created_at,
        "previewText": result.preview_text,
        "ackCount": result.ack_count,
        "expired": result.expired,
    })
}

fn agent_settings_api(settings: &AgentSettings) -> Value {
    let skills = builtin_skills::merged_catalog(
        &settings.builtin_skill_overrides,
        &settings.uploaded_skills,
    )
    .unwrap_or_else(|_| settings.uploaded_skills.clone());
    let uploaded_skills = settings
        .uploaded_skills
        .iter()
        .filter_map(|skill| {
            crate::resource_access::api_projection(
                skill,
                nola_core::agent::resource_access::AgentResourceKind::Skill,
            )
        })
        .collect::<Vec<_>>();
    json!({
        "enable_local_dev_tools": settings.enable_local_dev_tools,
        "allowed_dirs": settings.allowed_dirs,
        "bash_prefixes": settings.bash_prefixes,
        "bash_allowed_prefixes": settings.bash_prefixes,
        "agent_llm_endpoint": settings.agent_llm_endpoint,
        "agent_llm_model": settings.agent_llm_model,
        "temperature": settings.temperature,
        "top_p": settings.top_p,
        "max_output_tokens": settings.max_output_tokens,
        "max_context_tokens": settings.max_context_tokens,
        "response_style": settings.response_style,
        "custom_system_prompt": settings.custom_system_prompt,
        "long_term_memory": settings.long_term_memory,
        "user_preferences": settings.user_preferences,
        "mcp_servers": settings.mcp_servers.iter()
            .filter_map(crate::resource_access::mcp_api_projection)
            .collect::<Vec<_>>(),
        "skill_settings_backup_key": settings.skill_settings_backup_key,
        "skills": skills,
        "uploaded_skills": uploaded_skills,
        "uploaded_documents": crate::resource_access::data_source_api_projections(&settings.uploaded_documents),
        "network_policy": settings.network_policy,
        "permission_profile": settings.permission_profile,
        "has_agent_llm_api_key": !settings.agent_llm_api_key.trim().is_empty(),
        "agent_mode_enabled": settings.agent_mode_enabled,
        "host_tools_enabled": settings.host_tools_enabled,
        "execution_backends": crate::external_executor::discover_execution_backends(),
        "execution_backend": settings.execution_backend,
    })
}

fn validate_model_setting_values(value: &Value) -> Result<(), String> {
    if let Some(policy) = value.get("network_policy") {
        if !policy.as_str().is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "ask" | "deny" | "allow"
            )
        }) {
            return Err("network_policy must be 'ask', 'deny', or 'allow'".into());
        }
    }
    let mut request = value.clone();
    let object = request
        .as_object_mut()
        .ok_or("settings must be an object")?;
    for (alias, canonical) in [
        ("maxOutputTokens", "max_output_tokens"),
        ("maxContextTokens", "max_context_tokens"),
    ] {
        if let Some(alias_value) = object.get(alias).cloned() {
            if object
                .get(canonical)
                .is_some_and(|existing| existing != &alias_value)
            {
                return Err(format!("conflicting {canonical} values"));
            }
            object.insert(canonical.into(), alias_value);
        }
    }
    crate::runtime_config::RuntimeConfig::default()
        .with_model_request(&request)
        .map(|_| ())
}

fn merge_agent_settings(settings: &mut AgentSettings, value: Value) {
    let Some(map) = value.as_object() else {
        return;
    };
    if let Some(value) = bool_field(map, "enable_local_dev_tools") {
        settings.enable_local_dev_tools = value;
    }
    if let Some(value) = string_field(map, "execution_backend") {
        let available = crate::external_executor::discover_execution_backends();
        if available.iter().any(|backend| {
            backend.id == value
                && backend.availability
                    == nola_domain::execution_backend::ExecutionBackendAvailability::Available
        }) {
            settings.execution_backend = value;
        }
    }
    if let Some(value) = string_array_field(map, "allowed_dirs") {
        settings.allowed_dirs = value;
    }
    if let Some(value) = string_array_field(map, "bash_prefixes")
        .or_else(|| string_array_field(map, "bash_allowed_prefixes"))
        .or_else(|| string_array_field(map, "allowed_bash_prefixes"))
    {
        settings.bash_prefixes = value;
    }
    if let Some(value) = string_field(map, "network_policy") {
        settings.network_policy = normalize_network_policy_value(&value);
    }
    if let Some(value) = string_field(map, "permission_profile") {
        if let Some(profile) = crate::permissions::PermissionProfile::from_canonical_name(&value) {
            settings.permission_profile = profile.canonical_name().to_string();
            settings.network_policy = profile.network_policy().to_string();
        }
    }
    if let Some(value) = string_field(map, "response_style") {
        settings.response_style = value;
    }
    if let Some(value) = string_field(map, "responseStyle") {
        settings.response_style = value;
    }
    if let Some(value) = string_field(map, "custom_system_prompt")
        .or_else(|| string_field(map, "customSystemPrompt"))
    {
        settings.custom_system_prompt = value;
    }
    if let Some(value) = string_field(map, "long_term_memory") {
        settings.long_term_memory = value;
    }
    if let Some(value) = string_field(map, "user_preferences") {
        settings.user_preferences = value;
    }
    if let Some(value) = string_field(map, "agent_llm_endpoint") {
        settings.agent_llm_endpoint = value;
    }
    if let Some(value) = string_field(map, "agent_llm_api_key") {
        settings.agent_llm_api_key = value;
    }
    if let Some(value) = string_field(map, "agent_llm_model") {
        settings.agent_llm_model = value;
    }
    if let Some(value) = number_field(map, "temperature") {
        settings.temperature = value.clamp(0.0, 2.0);
    }
    if let Some(value) = number_field(map, "top_p").or_else(|| number_field(map, "topP")) {
        settings.top_p = value.clamp(0.0, 1.0);
    }
    if let Some(value) =
        unsigned_field(map, "max_output_tokens").or_else(|| unsigned_field(map, "maxOutputTokens"))
    {
        settings.max_output_tokens = value;
    }
    if let Some(value) = unsigned_field(map, "max_context_tokens")
        .or_else(|| unsigned_field(map, "maxContextTokens"))
    {
        settings.max_context_tokens = value;
    }
    if let Some(value) = map.get("mcp_servers").and_then(Value::as_array) {
        settings.mcp_servers = value.clone();
    }
    if let Some(value) = map.get("skills").and_then(Value::as_array) {
        let (overrides, uploaded) = builtin_skills::split_payload(value);
        settings.builtin_skill_overrides = overrides;
        settings.uploaded_skills = uploaded;
    } else if let Some(value) = map.get("uploaded_skills").and_then(Value::as_array) {
        let (overrides, uploaded) = builtin_skills::split_payload(value);
        if !overrides.is_empty() {
            settings.builtin_skill_overrides = overrides;
        }
        settings.uploaded_skills = uploaded;
    }
    if let Some(value) = map.get("uploaded_documents").and_then(Value::as_array) {
        settings.uploaded_documents = value.clone();
    }
    if let Some(value) = bool_field(map, "agent_mode_enabled") {
        settings.agent_mode_enabled = value;
    }
    if let Some(value) = bool_field(map, "host_tools_enabled") {
        settings.host_tools_enabled = value;
    }
}

fn string_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(ToOwned::to_owned)
}

fn number_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    map.get(key).and_then(Value::as_f64)
}

fn unsigned_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    map.get(key).and_then(Value::as_u64)
}

fn bool_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<bool> {
    map.get(key).and_then(Value::as_bool)
}

fn string_array_field(map: &serde_json::Map<String, Value>, key: &str) -> Option<Vec<String>> {
    map.get(key).and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn normalize_network_policy_value(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "ask" => "ask".to_string(),
        "deny" => "deny".to_string(),
        "allow" => "allow".to_string(),
        _ => "ask".to_string(),
    }
}

fn file_response(web_root: &Path, rel: &str) -> Response<Box<dyn Read + Send>> {
    let path = match safe_join(web_root, rel) {
        Some(path) => path,
        None => return boxed_response(StatusCode(403), "forbidden", "text/plain; charset=utf-8"),
    };
    let path = match fs::canonicalize(&path) {
        Ok(path) => path,
        Err(_) => return boxed_response(StatusCode(404), "not found", "text/plain; charset=utf-8"),
    };
    // Every caller serves public assets; the web root can also contain private
    // application files and must not itself be the exposure boundary.
    let allowed = fs::canonicalize(web_root.join("app/static"))
        .ok()
        .is_some_and(|root| path.starts_with(root))
        && !path
            .components()
            .any(|part| matches!(part, Component::Normal(name) if name == ".nola-host-secrets"));
    if !allowed {
        return boxed_response(StatusCode(403), "forbidden", "text/plain; charset=utf-8");
    }
    let file = match open_resolved_static_file(&path) {
        Ok(file) => file,
        Err(_) => return boxed_response(StatusCode(404), "not found", "text/plain; charset=utf-8"),
    };
    // Opening a directory succeeds on Unix, but streaming it fails after the
    // Content-Length header is sent and leaves browsers waiting for its body.
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return boxed_response(StatusCode(404), "not found", "text/plain; charset=utf-8");
    }
    let mime = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    Response::from_file(file)
        .boxed()
        .with_status_code(StatusCode(200))
        .with_header(content_type(&mime))
}

// The caller has resolved links and checked the public boundary. Do not reopen
// that path with ancestor symlink traversal: directories can change after the
// check. Walk from the filesystem root using pinned directory handles instead.
#[cfg(unix)]
fn open_resolved_static_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let mut directory = fs::File::open("/")?;
    let parts: Vec<_> = path.components().collect();
    if !path.is_absolute() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            if matches!(part, Component::RootDir) {
                continue;
            }
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
        };
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if index + 1 < parts.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // openat binds this component to the already opened directory, even
        // if another process renames that directory while the walk continues.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        directory = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(directory)
}

#[cfg(windows)]
fn open_resolved_static_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    if !path.is_absolute() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let mut current = PathBuf::new();
    let mut parents = Vec::new();
    for part in path.components() {
        current.push(part.as_os_str());
        if !matches!(part, Component::RootDir | Component::Normal(_)) {
            continue;
        }
        // Hold every ancestor without FILE_SHARE_DELETE until the final file
        // is open. Windows then refuses concurrent ancestor rename/replacement.
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&current)?;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        }
        parents.push(file);
    }
    parents
        .pop()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))
}

#[cfg(not(any(unix, windows)))]
fn open_resolved_static_file(_path: &Path) -> std::io::Result<fs::File> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) if part != ".nola-host-secrets" => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

fn json_response(status: StatusCode, value: Value) -> Response<Box<dyn Read + Send>> {
    boxed_response(
        status,
        &value.to_string(),
        "application/json; charset=utf-8",
    )
}

fn html_response(status: StatusCode, html: String) -> Response<Box<dyn Read + Send>> {
    boxed_response(status, &html, "text/html; charset=utf-8")
}

fn redirect_response(location: &str) -> Response<Box<dyn Read + Send>> {
    boxed_response(StatusCode(303), "", "text/plain; charset=utf-8")
        .with_header(Header::from_bytes(&b"Location"[..], location.as_bytes()).unwrap())
}

fn boxed_response(
    status: StatusCode,
    body: &str,
    content_type_value: &str,
) -> Response<Box<dyn Read + Send>> {
    Response::from_string(body.to_string())
        .boxed()
        .with_status_code(status)
        .with_header(content_type(content_type_value))
}

fn content_type(value: &str) -> Header {
    Header::from_bytes(&b"Content-Type"[..], value.as_bytes()).unwrap()
}

fn method_not_allowed() -> Response<Box<dyn Read + Send>> {
    json_response(StatusCode(405), json!({"error": "method not allowed"}))
}

fn not_found(message: &str) -> Response<Box<dyn Read + Send>> {
    json_response(StatusCode(404), json!({"error": message}))
}

fn with_local_first(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.insert("localFirst".to_string(), Value::Bool(true));
        map.insert("backend".to_string(), Value::String("cli-rust".to_string()));
    }
    value
}

fn normalize_title(value: Option<&str>) -> String {
    let title = value
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if title.is_empty() {
        "新对话".to_string()
    } else {
        truncate_chars(&title, 120)
    }
}

fn first_message_title(value: &str) -> Option<String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(truncate_chars(&normalized, 63))
    }
}

fn adopt_first_message_conversation_title_after_admission(
    identity:&nola_domain::remote_submission::RemoteSubmissionIdentity,
    request_digest:&str,
    visible_user_text:&str,
)->Result<bool,String>{
    adopt_first_message_conversation_title_at(&NolaStore::open_default()?,identity,request_digest,visible_user_text)
}

fn adopt_first_message_conversation_title_at(
    store:&NolaStore,
    identity:&nola_domain::remote_submission::RemoteSubmissionIdentity,
    request_digest:&str,
    visible_user_text:&str,
)->Result<bool,String>{
    let mut app=crate::application_port::CliApplicationPort::persistent(store.path())?;
    let receipt:nola_domain::remote_submission::SourceSubmissionReceipt=app.execute(
        nola_application::ApplicationCommand::ReadSourceSubmissionReceipt {identity:identity.clone(),request_digest:request_digest.into()})?;
    receipt.validate()?;
    if receipt.identity!=*identity || receipt.request_digest!=request_digest {return Err("title_admission_source_changed".into());}
    if receipt.phase!=nola_domain::remote_submission::RemoteSubmissionPhase::Admitted {return Ok(false);}
    let conversation_id=&identity.conversation_id;
    let Some(title)=first_message_title(visible_user_text) else {return Ok(false);};
    if !store.web_automatic_title_is_current(conversation_id)? {
        return Ok(false);
    }
    let Some(value) = store.get_web_conversation(conversation_id)? else {
        return Ok(false);
    };
    let mut conversation = serde_json::from_value::<WebConversation>(value)
        .map_err(|error| format!("parse web conversation failed: {error}"))?;
    if conversation.title_source != ConversationTitleSource::Automatic {
        return Ok(false);
    }
    conversation.title = title;
    conversation.title_source = ConversationTitleSource::FirstMessage;
    conversation.updated_at = now_string();
    if store.rename_web_conversation(conversation_id,&conversation.title,ConversationTitleSource::FirstMessage,conversation.revision)? {
        Ok(true)
    } else {
        Err(format!("web conversation {conversation_id} does not exist"))
    }
}

fn sanitize_meta(value: Value) -> Value {
    if value.is_object() {
        value
    } else {
        json!({})
    }
}

fn enabled_items_text_tokens(items: &[Value]) -> usize {
    items
        .iter()
        .filter(|item| enabled_bool(item.get("enabled"), true))
        .map(|item| item.get("content").and_then(Value::as_str).unwrap_or(""))
        .map(estimate_tokens)
        .sum()
}

fn enabled_bool(value: Option<&Value>, default: bool) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Some(Value::Number(value)) => value.as_i64().map(|value| value != 0).unwrap_or(default),
        Some(Value::Null) | None => default,
        _ => default,
    }
}

fn context_data_sources(uploaded_documents: &[Value], mcp_servers: &[Value]) -> Vec<Value> {
    let mut sources = Vec::new();
    for (index, item) in uploaded_documents.iter().enumerate() {
        let name = non_empty_string(item.get("name").and_then(Value::as_str).unwrap_or(""))
            .unwrap_or("Document")
            .to_string();
        let display_name = if name == "Document" {
            format!("Document {}", index + 1)
        } else {
            name
        };
        let kind = match item
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "folder" => "folder",
            "file" => "file",
            _ => "uploaded_document",
        };
        let content_chars = item
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .count();
        sources.push(json!({
            "id": format!("document:web:{index}:{}", stable_data_source_id_part(&display_name)),
            "name": display_name,
            "kind": kind,
            "location": "web",
            "capabilities": ["readable", "searchable", "contextual"],
            "privacy": {
                "scopes": ["local_storage", "allow_cloud_context"],
                "rawSamplesLeaveDevice": false,
            },
            "connectionState": if enabled_bool(item.get("enabled"), true) { "ready" } else { "disabled" },
            "metadata": {
                "characters": content_chars.to_string(),
                "origin": "web",
                "originIndex": index.to_string(),
            },
        }));
    }

    for (index, server) in mcp_servers.iter().enumerate() {
        let name = non_empty_string(server.get("name").and_then(Value::as_str).unwrap_or(""))
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("MCP {}", index + 1));
        let location = server
            .get("url")
            .or_else(|| server.get("endpoint"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("web");
        sources.push(json!({
            "id": format!("mcp:web:{index}:{}", stable_data_source_id_part(&name)),
            "name": name,
            "kind": "mcp",
            "location": location,
            "capabilities": ["executable", "contextual"],
            "privacy": {
                "scopes": ["allow_agent_tools", "requires_confirmation"],
                "rawSamplesLeaveDevice": false,
            },
            "connectionState": if enabled_bool(server.get("enabled"), true) { "ready" } else { "disabled" },
            "metadata": {
                "origin": "web",
                "originIndex": index.to_string(),
            },
        }));
    }
    sources
}

fn persisted_data_source_values(conversation_id: &str) -> Vec<Value> {
    with_data_source_store(|store| {
        Ok(store
            .resolved_sources(conversation_id)
            .into_iter()
            .map(|(source, resolution)| {
                // Listing metadata must not refresh content or mutate read state.
                // A token estimate is unknown until the request actually uses content.
                let token_estimate: Option<usize> = None;
                let connection_state = if resolution.usable {
                    "ready"
                } else if resolution.should_request_consent {
                    "consent_required"
                } else if resolution.should_refresh {
                    "refresh_required"
                } else {
                    "disabled"
                };
                json!({
                    "id": source.id,
                    "name": source.name,
                    "kind": source.kind,
                    "location": source.location,
                    "accessPolicy": source.access_policy,
                    "capabilities": source.capabilities,
                    "privacy": {
                        "scopes": ["conversation_context"],
                        "rawSamplesLeaveDevice": false,
                    },
                    "connectionState": connection_state,
                    "resolution": resolution,
                    "validationContract": source.validation_contract,
                    "metadata": {
                        "origin": "cli-host",
                        "tokenEstimate": token_estimate,
                        "recordCount": source.validation_contract.record_count,
                    },
                })
            })
            .collect::<Vec<_>>())
    })
    .unwrap_or_default()
}

fn context_pack_value(
    conversation_id: &str,
    data_sources: &[Value],
    uploaded_documents: &[Value],
    mcp_servers: &[Value],
) -> Value {
    let captured_at = Local::now().timestamp_millis();
    let mut snapshots = Vec::new();
    for source in data_sources {
        if source
            .get("connectionState")
            .and_then(Value::as_str)
            .unwrap_or("ready")
            != "ready"
        {
            continue;
        }
        let source_id = source.get("id").and_then(Value::as_str).unwrap_or("");
        if source_id.is_empty() {
            continue;
        }
        let origin_index = source
            .get("metadata")
            .and_then(|meta| meta.get("originIndex"))
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let source_kind = source.get("kind").and_then(Value::as_str).unwrap_or("");
        let token_estimate = if matches!(source_kind, "file" | "folder" | "uploaded_document") {
            uploaded_documents
                .get(origin_index)
                .and_then(|item| item.get("content"))
                .and_then(Value::as_str)
                .map(|content| estimate_tokens(&truncate_chars(content, 4000)))
                .unwrap_or(0)
        } else if source_kind == "mcp" {
            mcp_servers
                .get(origin_index)
                .map(|item| estimate_tokens(&item.to_string()))
                .unwrap_or(0)
        } else {
            source
                .get("metadata")
                .and_then(|metadata| metadata.get("tokenEstimate"))
                .and_then(Value::as_u64)
                .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
                .unwrap_or(0)
        };
        snapshots.push(json!({
            "dataSourceId": source_id,
            "itemCount": 1,
            "tokenEstimate": token_estimate,
            "capturedAt": captured_at,
            "summary": source.get("name").and_then(Value::as_str).unwrap_or(""),
        }));
    }
    let token_estimate = snapshots
        .iter()
        .filter_map(|item| item.get("tokenEstimate").and_then(Value::as_u64))
        .sum::<u64>();
    json!({
        "id": format!("context-pack:{conversation_id}"),
        "dataSourceIds": snapshots
            .iter()
            .filter_map(|item| item.get("dataSourceId").and_then(Value::as_str))
            .collect::<Vec<_>>(),
        "snapshots": snapshots,
        "tokenEstimate": token_estimate,
        "createdAt": captured_at,
    })
}

fn stable_data_source_id_part(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.trim().to_lowercase().chars() {
        let keep = ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&ch);
        if keep {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.chars().count() >= 48 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

#[allow(clippy::too_many_arguments)]
fn context_suggestion_details(
    percentage: usize,
    total_tokens: usize,
    estimated_limit: usize,
    active_message_tokens: usize,
    session_memory_tokens: usize,
    long_term_memory_tokens: usize,
    user_preferences_tokens: usize,
    mcp_servers_tokens: usize,
    uploaded_skills_tokens: usize,
    uploaded_documents_tokens: usize,
    data_sources_tokens: usize,
    scaffold_tokens: usize,
    message_count: usize,
    archived_count: usize,
) -> Vec<Value> {
    let total_context_tokens = total_tokens.max(1);
    let active_share = token_share(active_message_tokens, total_context_tokens);
    let scaffold_share = token_share(scaffold_tokens, total_context_tokens);
    let uploaded_context_tokens = uploaded_skills_tokens + uploaded_documents_tokens;
    let data_source_context_tokens = if data_sources_tokens > 0 {
        data_sources_tokens
    } else {
        mcp_servers_tokens + uploaded_documents_tokens
    };
    let mut details = Vec::new();

    if percentage >= 90 {
        details.push(context_suggestion(
            "high",
            ContextSuggestionCode::NearLimit,
            json!({"used": total_tokens, "limit": estimated_limit}),
        ));
    } else if percentage >= 75 {
        details.push(context_suggestion(
            "medium",
            ContextSuggestionCode::HighWater,
            json!({"used": total_tokens, "limit": estimated_limit}),
        ));
    }
    if active_message_tokens >= 4_000 && active_share >= 45 {
        details.push(context_suggestion(
            "medium",
            ContextSuggestionCode::ActiveMessages,
            json!({"share": active_share, "count": message_count}),
        ));
    }
    if session_memory_tokens >= 5_000 {
        details.push(context_suggestion(
            "medium",
            ContextSuggestionCode::SessionMemory,
            json!({}),
        ));
    }
    if uploaded_context_tokens >= 4_000 {
        details.push(context_suggestion(
            "medium",
            ContextSuggestionCode::UploadedContext,
            json!({}),
        ));
    }
    if data_source_context_tokens >= 4_500 {
        details.push(context_suggestion(
            "medium",
            ContextSuggestionCode::DataSources,
            json!({}),
        ));
    }
    if long_term_memory_tokens >= 2_000 {
        details.push(context_suggestion(
            "low",
            ContextSuggestionCode::LongTermMemory,
            json!({}),
        ));
    }
    if user_preferences_tokens >= 1_500 {
        details.push(context_suggestion(
            "low",
            ContextSuggestionCode::UserPreferences,
            json!({}),
        ));
    }
    if mcp_servers_tokens >= 2_500 {
        details.push(context_suggestion(
            "low",
            ContextSuggestionCode::McpDescriptions,
            json!({}),
        ));
    }
    if scaffold_tokens >= 3_000 && scaffold_share >= 20 {
        details.push(context_suggestion(
            "info",
            ContextSuggestionCode::Scaffold,
            json!({"share": scaffold_share}),
        ));
    }
    if archived_count > 0 {
        details.push(context_suggestion(
            "info",
            ContextSuggestionCode::ArchivedMessages,
            json!({"count": archived_count}),
        ));
    }
    if details.is_empty() && percentage <= 55 {
        details.push(context_suggestion(
            "info",
            ContextSuggestionCode::Healthy,
            json!({}),
        ));
    }
    details
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ContextSuggestionCode {
    NearLimit,
    HighWater,
    ActiveMessages,
    SessionMemory,
    UploadedContext,
    DataSources,
    LongTermMemory,
    UserPreferences,
    McpDescriptions,
    Scaffold,
    ArchivedMessages,
    Healthy,
}

fn context_suggestion(severity: &str, code: ContextSuggestionCode, parameters: Value) -> Value {
    json!({
        "severity": severity,
        "code": code,
        "parameters": parameters,
    })
}

#[cfg(test)]
mod context_suggestion_contract_tests {
    use super::*;

    #[test]
    fn suggestions_transport_semantics_and_numeric_parameters_not_ui_text() {
        let mut rows = context_suggestion_details(
            95, 20_000, 21_000, 10_000, 5_000, 2_000, 1_500, 2_500, 4_000, 5_000, 5_000, 4_000, 30,
            8,
        );
        rows.extend(context_suggestion_details(
            80, 800, 1_000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ));
        rows.extend(context_suggestion_details(
            20, 200, 1_000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ));
        let codes: std::collections::HashSet<_> = rows
            .iter()
            .map(|row| row["code"].as_str().unwrap())
            .collect();
        assert_eq!(codes.len(), 12);
        for row in &rows {
            assert!(row.get("title").is_none());
            assert!(row.get("detail").is_none());
            assert!(row.get("estimatedSavingsTokens").is_none());
            assert!(row["parameters"]
                .as_object()
                .unwrap()
                .values()
                .all(Value::is_u64));
        }
        let near = rows.iter().find(|row| row["code"] == "near_limit").unwrap();
        assert_eq!(near["parameters"], json!({"used": 20_000, "limit": 21_000}));
        let archived = rows
            .iter()
            .find(|row| row["code"] == "archived_messages")
            .unwrap();
        assert_eq!(archived["parameters"], json!({"count": 8}));
    }
}

fn token_share(tokens: usize, total: usize) -> usize {
    if total == 0 {
        0
    } else {
        ((tokens as f64 / total as f64) * 100.0).round() as usize
    }
}

fn estimate_tokens(value: &str) -> usize {
    if value.is_empty() {
        return 0;
    }
    estimate_json_values_tokens(&[Value::String(value.to_string())])
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut out = value.chars().take(limit).collect::<String>();
    if value.chars().count() > limit {
        out.push('…');
    }
    out
}

fn safe_filename_part(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            ch if ch.is_control() => '-',
            ch => ch,
        })
        .collect::<String>();
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    out = out.trim_matches(['.', ' ', '-']).to_string();
    if out.is_empty() {
        "chat".to_string()
    } else {
        truncate_chars(&out, 80)
    }
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    for pair in query.split('&') {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        if raw_key == key {
            return Some(percent_decode(raw_value));
        }
    }
    None
}

fn percent_decode(value: &str) -> String {
    let mut bytes = Vec::new();
    let raw = value.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'+' {
            bytes.push(b' ');
            index += 1;
        } else if raw[index] == b'%' && index + 2 < raw.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                bytes.push(byte);
                index += 3;
            } else {
                bytes.push(raw[index]);
                index += 1;
            }
        } else {
            bytes.push(raw[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

fn sse_event(value: Value) -> Vec<u8> {
    format!("data: {}\n\n", value).into_bytes()
}

fn new_id(prefix: &str) -> String {
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{}-{}-{}",
        prefix,
        Local::now().format("%Y%m%d%H%M%S%3f"),
        counter
    )
}

fn now_string() -> String {
    Local::now().to_rfc3339()
}

fn remote_web_host_route_allowed(method: &Method, path: &str) -> bool {
    if method == &Method::Post && path == "/api/mobile/transport/tailcat" { return true; }

    matches!(
        (method, path),
        (&Method::Get, "/api/health")
            | (&Method::Head, "/api/health")
            | (&Method::Get, "/api/web/session")
            | (&Method::Head, "/api/web/session")
            | (&Method::Post, "/api/web/login")
            | (&Method::Get, "/login")
            | (&Method::Head, "/login")
            | (&Method::Post, "/login")
            | (&Method::Get, "/favicon.ico")
            | (&Method::Head, "/favicon.ico")
            | (&Method::Get, "/apple-touch-icon.png")
            | (&Method::Head, "/apple-touch-icon.png")
            | (&Method::Get, "/apple-touch-icon-precomposed.png")
            | (&Method::Head, "/apple-touch-icon-precomposed.png")
    ) || path.starts_with("/api/mobile/")
        || (matches!(method, &Method::Get | &Method::Head)
            && path.starts_with("/auth/mobile-handoff/"))
        || (matches!(method, &Method::Get | &Method::Head) && path.starts_with("/static/"))
        || (matches!(method, &Method::Get | &Method::Head)
            && path.starts_with("/ui/assets/")
            && !path.contains('\\')
            && !path.split('/').any(|part| part == ".."))
}

fn error_html(code: u16, message: &str) -> String {
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{code}</title><h1>{code}</h1><p>{}</p>",
        html_escape(message)
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    #[test]
    fn user_instruction_producer_keeps_explicit_style_and_custom_bytes() {
        let settings = super::AgentSettings { response_style:"casual".into(), custom_system_prompt:"  Preserve exactly.\n第二行  ".into(), ..Default::default() };
        assert_eq!(super::web_user_instruction_values(&serde_json::json!({"responseStyle":"formal"}),&settings),
            vec![("web-response-style","formal".into()),("web-custom-prompt","Preserve exactly.\n第二行".into())]);
        assert_eq!(super::web_user_instruction_values(&serde_json::json!({}),&settings)[0],("web-response-style","casual".into()));
    }

    #[test]
    fn invalid_network_policy_never_widens_permissions() {
        for value in [
            serde_json::json!("typo"),
            serde_json::json!(""),
            serde_json::json!(true),
            serde_json::json!(null),
        ] {
            assert!(super::validate_model_setting_values(
                &serde_json::json!({"network_policy": value})
            )
            .is_err());
        }
        for value in ["ask", "deny", "allow", " ASK "] {
            assert!(super::validate_model_setting_values(
                &serde_json::json!({"network_policy": value})
            )
            .is_ok());
        }
        assert_eq!(super::normalize_network_policy_value("typo"), "ask");
    }
    use super::{
        agent_context_messages_from_request, apply_model_selection, build_script_metadata,
        chat_conversation_route, clean_html_text, context_data_sources, context_pack_value,
        first_message_title, html_meta_content, html_title,
        load_or_create_web_host_access_token_at, merge_agent_settings,
        native_browser_tool_names_from_status, native_host_can_claim_command,
        native_host_id_is_valid, native_host_manifest_from_poll, native_optional_host_id,
        native_result_identity_matches, native_unix_time_ms, native_wait_sec, new_id,
        non_negative_int_field, parse_accept_language, percent_decode, permission_rule_route,
        positive_usize_field, prepare_web_chat_context, read_text_preview,
        remote_web_host_route_allowed,
        request_exact_web_agent_cancel, resolve_local_readable_path, result_detail_route,
        safe_filename_part, safe_join, scheduled_action_route, sorted_conversation_messages,
        stable_data_source_id_part, try_register_web_agent_run,
        valid_mobile_access_token, validate_script_lines, web_context_components_from_request,
        web_event_is_core_projection, AgentSettings, ContextComponentKind, ConversationTitleSource,
        NativeBridgeKind, NativeCommand, NativeCommandStatus, NativeHost,
        WebAgentCancellationStatus, WebConversation, WebMessage, WebPermissionRule, WebState,
        RemoteSubmissionIdentity, ObservedWorkerOwner, OBSERVED_WORKERS,
    };
    use crate::web::valid_host_identity;
    use nola_core::context_engine::{
        prepare_context, ContextBudgetSpec, ContextPlatform, ContextPrepareAction,
        ContextPrepareInput,
    };
    use serde_json::json;
    use serde_json::Value;
    use std::fs;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::path::Path;
    use std::time::Instant;
    use tiny_http::Method;

    #[test]
    fn safe_join_rejects_parent_components() {
        assert!(safe_join(Path::new("/tmp/web"), "app/static/favicon.png").is_some());
        assert!(safe_join(Path::new("/tmp/web"), "../secret").is_none());
        assert!(safe_join(Path::new("/tmp/web"), "/etc/passwd").is_none());
        assert!(safe_join(
            Path::new("/tmp/web"),
            "app/static/.nola-host-secrets/key.sqlite"
        )
        .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn static_files_refuse_symlinked_vault_even_inside_web_root() {
        let root = tempfile::tempdir().unwrap();
        let public = root.path().join("app/static");
        std::fs::create_dir_all(&public).unwrap();
        let vault = public.join(".nola-host-secrets");
        std::fs::create_dir(&vault).unwrap();
        std::fs::write(vault.join("key.sqlite"), b"private fixture").unwrap();
        std::os::unix::fs::symlink(vault.join("key.sqlite"), public.join("public.sqlite")).unwrap();
        assert_eq!(
            super::file_response(root.path(), "app/static/public.sqlite")
                .status_code()
                .0,
            403
        );
    }

    #[cfg(unix)]
    #[test]
    fn static_files_refuse_symlinks_outside_web_root_but_serve_local_assets() {
        let root = tempfile::tempdir().unwrap();
        let public = root.path().join("app/static");
        std::fs::create_dir_all(&public).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), b"private fixture").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), public.join("public.txt"))
            .unwrap();
        assert_eq!(
            super::file_response(root.path(), "app/static/public.txt")
                .status_code()
                .0,
            403
        );
        std::fs::write(public.join("asset.txt"), b"public fixture").unwrap();
        assert_eq!(
            super::file_response(root.path(), "app/static/asset.txt")
                .status_code()
                .0,
            200
        );
        assert_eq!(
            super::file_response(root.path(), "app/static")
                .status_code()
                .0,
            404
        );
        std::fs::create_dir(public.join("assets")).unwrap();
        assert_eq!(
            super::file_response(root.path(), "app/static/assets")
                .status_code()
                .0,
            404
        );
        use std::io::Read;
        let mut body = String::new();
        super::file_response(root.path(), "app/static/asset.txt")
            .into_reader()
            .read_to_string(&mut body)
            .unwrap();
        assert_eq!(body, "public fixture");
        for missing in ["app/static/missing.txt", "app/static/%2e%2e/secret"] {
            let response = super::file_response(root.path(), missing);
            assert_eq!(response.status_code().0, 404);
            let mut body = String::new();
            response.into_reader().read_to_string(&mut body).unwrap();
            assert_eq!(body, "not found");
        }
        std::os::unix::fs::symlink(public.join("asset.txt"), public.join("alias.txt")).unwrap();
        assert_eq!(
            super::file_response(root.path(), "app/static/alias.txt")
                .status_code()
                .0,
            200
        );

        // A special file must be rejected before a blocking read-only open.
        // Keep the regression bounded even if that open is reintroduced.
        use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
        let fifo = public.join("stream.pipe");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (send, receive) = std::sync::mpsc::channel();
        let directory = root.path().to_path_buf();
        let worker = std::thread::spawn(move || {
            send.send(
                super::file_response(&directory, "app/static/stream.pipe")
                    .status_code()
                    .0,
            )
            .unwrap();
        });
        let status = receive.recv_timeout(std::time::Duration::from_secs(2));
        let unblock = status.is_err().then(|| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&fifo)
                .unwrap()
        });
        worker.join().unwrap();
        drop(unblock);
        assert_eq!(
            status.expect("static FIFO response blocked waiting for a writer"),
            404
        );
    }

    #[cfg(unix)]
    #[test]
    fn static_files_refuse_symlinked_non_public_file_inside_web_root() {
        let root = tempfile::tempdir().unwrap();
        let public = root.path().join("app/static");
        std::fs::create_dir_all(&public).unwrap();
        let private = root.path().join("private.env");
        std::fs::write(&private, b"fictional private fixture").unwrap();
        std::os::unix::fs::symlink(&private, public.join("public.txt")).unwrap();
        assert_eq!(
            super::file_response(root.path(), "app/static/public.txt")
                .status_code()
                .0,
            403
        );
        assert_eq!(
            super::file_response(root.path(), "private.env")
                .status_code()
                .0,
            403
        );
    }

    #[cfg(unix)]
    #[test]
    fn static_files_refuse_ancestor_replacement_after_resolution() {
        use std::io::Read;
        let root = tempfile::tempdir().unwrap();
        let public = root.path().join("app/static");
        let assets = public.join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("item.txt"), "public fixture").unwrap();
        let resolved = std::fs::canonicalize(assets.join("item.txt")).unwrap();
        let private = tempfile::tempdir().unwrap();
        std::fs::write(private.path().join("item.txt"), "private fixture").unwrap();
        // Reproduce the exact check/open window, without timing-dependent loops.
        std::fs::rename(&assets, public.join("original-assets")).unwrap();
        std::os::unix::fs::symlink(private.path(), &assets).unwrap();
        assert_eq!(std::fs::read_to_string(&resolved).unwrap(), "private fixture");
        assert!(super::open_resolved_static_file(&resolved).is_err());
        // The already resolved target of a legitimate public directory alias
        // still works; we do not prohibit links in the deployed asset layout.
        std::fs::remove_file(&assets).unwrap();
        std::os::unix::fs::symlink(public.join("original-assets"), &assets).unwrap();
        let response = super::file_response(root.path(), "app/static/assets/item.txt");
        assert_eq!(response.status_code().0, 200);
        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        assert_eq!(body, "public fixture");
    }

    #[test]
    fn first_message_title_normalizes_and_bounds_visible_text() {
        assert_eq!(
            first_message_title("  Analyze   this report\ncarefully  ").as_deref(),
            Some("Analyze this report carefully")
        );
        assert_eq!(first_message_title(" \n\t "), None);
        assert_eq!(
            first_message_title(&"a".repeat(100))
                .expect("non-empty title")
                .chars()
                .count(),
            64
        );
    }

    #[test]
    fn model_selection_rejects_removed_official_alias() {
        let mut conversation = WebConversation {
            configuration_revision: 1,
            revision: 1,
            is_pinned: false,
            conversation_id: "conversation-current".to_string(),
            title: "Current".to_string(),
            title_source: ConversationTitleSource::User,
            session_memory: String::new(),
            knowledge_points: String::new(),
            wiki: String::new(),
            last_compacted_at: None,
            last_context_usage: None,
            selected_model_source: "auto".to_string(),
            selected_model_id: String::new(),
            execution_backend: "nola".to_string(),
            external_execution_sessions: Default::default(),
            created_at: "2026-08-21T00:00:00Z".to_string(),
            updated_at: "2026-08-21T00:00:00Z".to_string(),
        };

        let error = apply_model_selection(&mut conversation, Some("official"), Some("model"))
            .expect_err("removed model-source aliases must not be accepted");
        assert_eq!(error, "selectedModelSource must be auto, cloud, or local");
        assert_eq!(conversation.selected_model_source, "auto");
    }

    #[test]
    fn web_agent_cancel_requests_the_registered_core_execution() {
        let conversation_id = new_id("web-cancel-test");
        let cancellation = crate::runtime_control::RuntimeExecutionControl::default();
        let _registration = try_register_web_agent_run(&conversation_id, cancellation.clone()).unwrap();
        let identity = RemoteSubmissionIdentity { source_host_id: "fixture-host".into(), client_id: "fixture-client".into(),
            conversation_id: conversation_id.clone(), submission_id: "run".into(), user_message_id: "user".into(), assistant_message_id: "assistant".into() };
        assert!(request_exact_web_agent_cancel(&identity).is_err(), "starting worker is not a proved owner");
        OBSERVED_WORKERS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap().insert(cancellation.execution_id(),
            ObservedWorkerOwner {conversation_id,run_id:identity.submission_id.clone(),assistant_message_id:identity.assistant_message_id.clone(),
                client_id:identity.client_id.clone(),generation:cancellation.execution_id()});
        let mut stale = identity.clone();
        stale.submission_id = "previous-run".into();
        assert!(request_exact_web_agent_cancel(&stale).is_err());
        stale = identity.clone(); stale.client_id = "different-client".into();
        assert!(request_exact_web_agent_cancel(&stale).is_err());
        assert!(!cancellation.is_requested());
        let status = request_exact_web_agent_cancel(&identity).unwrap();
        assert!(status.cancel_requested);
        assert!(!status.termination_confirmed);
        assert!(cancellation.is_requested());
        assert_eq!(
            request_exact_web_agent_cancel(&RemoteSubmissionIdentity {conversation_id:"missing-web-agent-run".into(),..identity}).unwrap(),
            WebAgentCancellationStatus {
                cancel_requested: false,
                termination_confirmed: false,
            }
        );
    }

    #[test]
    fn web_agent_reconnect_cannot_replace_an_active_run_implicitly() {
        let conversation_id = new_id("web-run-owner-test");
        let first = crate::runtime_control::RuntimeExecutionControl::default();
        let second = crate::runtime_control::RuntimeExecutionControl::default();
        let _registration = try_register_web_agent_run(&conversation_id, first.clone())
            .expect("first run registration");

        let active_execution_id =
            try_register_web_agent_run(&conversation_id, second.clone())
                .expect_err("a reconnect must not replace the active run");

        assert_eq!(active_execution_id, first.execution_id());
        assert!(!first.is_requested());
        assert!(!second.is_requested());
    }

    #[test]
    fn parses_chat_conversation_routes() {
        assert_eq!(
            chat_conversation_route("/api/chat/conversations/abc/messages"),
            Some(("abc", "messages"))
        );
        assert_eq!(
            chat_conversation_route("/api/chat/conversations/abc"),
            Some(("abc", ""))
        );
        assert_eq!(
            chat_conversation_route("/api/chat/conversations/abc/traces"),
            Some(("abc", "traces"))
        );
        assert_eq!(
            chat_conversation_route("/api/chat/conversations/abc/goal"),
            Some(("abc", "goal"))
        );
    }

    #[test]
    fn decodes_query_values() {
        assert_eq!(percent_decode("hello+%E4%B8%96%E7%95%8C"), "hello 世界");
    }

    #[test]
    fn builds_context_data_sources_and_pack() {
        let documents = vec![serde_json::json!({
            "name": "项目说明",
            "kind": "file",
            "content": "Nola 客户端本地优先。",
            "enabled": true,
        })];
        let mcp_servers = vec![serde_json::json!({
            "name": "Docs MCP",
            "endpoint": "https://mcp.example.com/docs",
            "description": "文档 MCP",
            "enabled": true,
        })];

        let sources = context_data_sources(&documents, &mcp_servers);

        assert_eq!(stable_data_source_id_part("项目说明!!!"), "项目说明");
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0]["name"], "项目说明");
        assert_eq!(sources[0]["kind"], "file");
        assert_eq!(
            sources[0]["capabilities"],
            serde_json::json!(["readable", "searchable", "contextual"])
        );
        assert_eq!(sources[0]["privacy"]["rawSamplesLeaveDevice"], false);
        assert_eq!(sources[0]["metadata"]["originIndex"], "0");
        assert_eq!(sources[1]["kind"], "mcp");

        let pack = context_pack_value("conv1", &sources, &documents, &mcp_servers);

        assert_eq!(pack["id"], "context-pack:conv1");
        assert_eq!(pack["dataSourceIds"].as_array().unwrap().len(), 2);
        assert!(pack["tokenEstimate"].as_u64().unwrap() > 0);
        assert_eq!(pack["snapshots"][0]["dataSourceId"], sources[0]["id"]);
    }

    #[test]
    fn web_core_context_retains_messages_and_settings() {
        let settings = AgentSettings {
            response_style: "concise".to_string(),
            user_preferences: "prefer Chinese answers".to_string(),
            ..AgentSettings::default()
        };
        let request_messages = (0..30)
            .map(|index| serde_json::json!({"role": "user", "content": format!("m{index}")}))
            .collect::<Vec<_>>();
        let body = serde_json::json!({
            "timezone": "Asia/Shanghai",
            "language": "zh-CN",
            "messages": request_messages,
        });

        let messages = agent_context_messages_from_request(&body, &settings);
        let components = web_context_components_from_request(&body, &settings).unwrap();
        assert!(components.iter().all(|component| {
            component.kind != nola_core::context_engine::ContextComponentKind::System
                && component.id != "web-system"
        }));
        let prepared = prepare_context(&ContextPrepareInput {
            platform: ContextPlatform::Web,
            components,
            messages,
            budget: ContextBudgetSpec {
                context_window_tokens: Some(128_000),
                reserved_output_tokens: 4_096,
                keep_recent_messages: 8,
                ..ContextBudgetSpec::default()
            },
            ..ContextPrepareInput::default()
        });
        assert_eq!(
            prepared.action,
            nola_core::context_engine::ContextPrepareAction::Ready
        );
        let retained = prepared
            .messages
            .iter()
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(retained.contains("Asia/Shanghai"));
        assert!(retained.contains("concise"));
        assert!(retained.contains("m0"));
        assert!(retained.contains("m29"));
    }

    #[test]
    fn web_context_preserves_typed_session_memory_separately_from_long_term_memory() {
        use nola_core::context_engine::ContextComponentKind;
        let settings = AgentSettings {
            long_term_memory: "durable fixture".into(),
            ..AgentSettings::default()
        };
        let components = web_context_components_from_request(
            &serde_json::json!({
                "context_components":[{"kind":"session_memory","id":"summary",
                    "content":"current conversation fixture"}]
            }),
            &settings,
        )
        .unwrap();
        assert!(components
            .iter()
            .any(|item| item.kind == ContextComponentKind::SessionMemory
                && item.content == "current conversation fixture"));
        assert!(components
            .iter()
            .any(|item| item.kind == ContextComponentKind::Memory
                && item.content == "durable fixture"));
    }

    #[test]
    fn web_context_cannot_restore_client_skill_components_after_runtime_revocation() {
        let disabled =
            crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(Vec::new()));
        let body = serde_json::json!({"context_components":[
            {"kind":"skill","id":"stale-skill","content":"revoked instructions"},
            {"kind":"session_memory","id":"summary","content":"retained memory"}
        ]});
        let components = super::web_context_components_with_retrieval(
            &body,
            &AgentSettings::default(),
            &disabled,
            true,
        )
        .unwrap();
        assert!(components
            .iter()
            .all(|component| component.kind != ContextComponentKind::Skill));
        assert!(components.iter().any(|component| component.kind
            == ContextComponentKind::SessionMemory
            && component.content == "retained memory"));
    }

    #[test]
    fn web_skill_context_uses_the_owned_catalog_prompt_without_resource_paths() {
        let fixture = tempfile::tempdir().unwrap();
        let mut contexts = Vec::new();
        for id in ["runtime-skill-a", "runtime-skill-b"] {
            let root = fixture.path().join(id);
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join("SKILL.md"), format!(
                "---\nname: {id}\ndescription: isolated catalog metadata\n---\nPrivate instructions.\n"
            )).unwrap();
            contexts.push(
                crate::skill_runtime::SkillRuntimeContext::from_configured_dirs(Some(vec![root])),
            );
        }
        for (index, expected) in ["runtime-skill-a", "runtime-skill-b"]
            .into_iter()
            .enumerate()
        {
            let components = super::web_context_components_with_retrieval(
                &serde_json::json!({}),
                &AgentSettings::default(),
                &contexts[index],
                true,
            )
            .unwrap();
            let skills = components
                .iter()
                .filter(|component| component.kind == ContextComponentKind::Skill)
                .collect::<Vec<_>>();
            assert_eq!(skills.len(), 1);
            assert_eq!(skills[0].content, contexts[index].catalog_prompt().unwrap());
            let content = skills[0].content.as_str().unwrap();
            assert!(content.contains(expected));
            assert!(!content.contains(if index == 0 {
                "runtime-skill-b"
            } else {
                "runtime-skill-a"
            }));
            assert!(!content.contains(fixture.path().to_str().unwrap()));
            assert!(!content.contains("base_directory"));
            assert!(!content.contains("Private instructions"));
        }
    }

    #[test]
    fn web_context_requires_summary_without_splitting_tool_transaction() {
        let mut messages = vec![
            serde_json::json!({"id": "u0", "role": "user", "content": "start"}),
            serde_json::json!({
                "id": "a0",
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    {"id": "call-a", "type": "function", "function": {"name": "read_file", "arguments": "{}"}},
                    {"id": "call-b", "type": "function", "function": {"name": "web_search", "arguments": "{}"}}
                ]
            }),
            serde_json::json!({"id": "t0", "role": "tool", "tool_call_id": "call-a", "content": "file result"}),
            serde_json::json!({"id": "t1", "role": "tool", "tool_call_id": "call-b", "content": "search result"}),
        ];
        messages.extend((0..12).map(|index| {
            serde_json::json!({
                "id": format!("later-{index}"),
                "role": "user",
                "content": "x".repeat(2_000),
            })
        }));

        let prepared = prepare_web_chat_context(&messages, &[], 4_096, 512, 4, 1_400, 900);

        assert_eq!(prepared.action, ContextPrepareAction::SummaryRequired);
        let archive = prepared.archive_request.expect("cautious archive request");
        let archived_ids = archive
            .messages
            .iter()
            .filter_map(|message| message.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(archived_ids.contains(&"a0"));
        assert!(archived_ids.contains(&"t0"));
        assert!(archived_ids.contains(&"t1"));
        assert_eq!(
            prepared
                .messages
                .last()
                .and_then(|message| message.get("id"))
                .and_then(Value::as_str),
            Some("later-11")
        );
    }

    #[test]
    fn parses_positive_iteration_limit_from_canonical_field() {
        assert_eq!(
            positive_usize_field(
                &serde_json::json!({"max_iterations": "7"}),
                "max_iterations"
            ),
            Some(7)
        );
        assert_eq!(
            positive_usize_field(&serde_json::json!({"maxIterations": 7}), "max_iterations"),
            None
        );
        assert_eq!(
            positive_usize_field(&serde_json::json!({"max_iterations": 0}), "max_iterations"),
            None
        );
    }

    #[test]
    fn parses_non_negative_task_accounting_fields() {
        assert_eq!(
            non_negative_int_field(&serde_json::json!({}), "requests", 1),
            Ok(1)
        );
        assert_eq!(
            non_negative_int_field(&serde_json::json!({"requests": "3"}), "requests", 1),
            Ok(3)
        );
        assert!(
            non_negative_int_field(&serde_json::json!({"requests": -1}), "requests", 1)
                .unwrap_err()
                .contains("non-negative")
        );
        assert!(
            non_negative_int_field(&serde_json::json!({"requests": "x"}), "requests", 1)
                .unwrap_err()
                .contains("integer")
        );
    }

    #[test]
    fn validates_native_bridge_host_ids_and_wait_zero() {
        assert!(native_host_id_is_valid("chrome.default:1"));
        assert!(!native_host_id_is_valid("bad host"));
        assert!(!native_host_id_is_valid("bad\nhost"));
        assert_eq!(
            native_optional_host_id(
                &serde_json::json!({"hostId": "chrome-default"}),
                &["hostId"]
            )
            .unwrap(),
            Some("chrome-default".to_string())
        );
        assert!(
            native_optional_host_id(&serde_json::json!({"hostId": "bad host"}), &["hostId"])
                .is_err()
        );
        assert_eq!(
            native_wait_sec(&serde_json::json!({"wait_sec": 0}), 25.0),
            0.0
        );
    }

    #[test]
    fn native_host_manifest_rejects_empty_capabilities_and_expiry() {
        let now = native_unix_time_ms();
        let base = serde_json::json!({
            "manifest_id": "native:test:instance",
            "manifest_version": 1,
            "instance_id": "instance",
            "issued_at_ms": now,
            "expires_at_ms": now + 60_000,
            "capabilities": []
        });
        assert!(native_host_manifest_from_poll(&base).is_err());
        let mut explicit = base;
        explicit["capabilities"] = serde_json::json!(["status"]);
        explicit["expires_at_ms"] = serde_json::json!(now);
        assert!(native_host_manifest_from_poll(&explicit).is_err());
        explicit["expires_at_ms"] = serde_json::json!(now + 60_000);
        assert!(native_host_manifest_from_poll(&explicit).is_ok());
    }

    #[test]
    fn native_browser_host_requires_exact_action_capability() {
        let now = native_unix_time_ms();
        let host = NativeHost {
            last_seen: Instant::now(),
            capabilities: vec!["status".to_string()],
            manifest_id: "manifest".to_string(),
            manifest_version: 1,
            instance_id: "instance".to_string(),
            issued_at_ms: now,
            expires_at_ms: now + 60_000,
        };
        let mut command = NativeCommand {
            id: "browser-command".to_string(),
            kind: NativeBridgeKind::Browser,
            action: "click".to_string(),
            args: serde_json::json!({}),
            preferred_host_id: None,
            created_at: Instant::now(),
            status: NativeCommandStatus::Pending,
            host_id: None,
            host_instance_id: None,
            host_manifest_id: None,
            host_manifest_expires_at_ms: None,
            claimed_at: None,
            result: None,
        };
        assert!(!native_host_can_claim_command("browser", &host, &command));
        command.action = "status".to_string();
        assert!(native_host_can_claim_command("browser", &host, &command));
    }

    #[test]
    fn native_browser_settings_advertise_only_exact_connected_tools() {
        let status = serde_json::json!({
            "connected": true,
            "hosts": [
                {"host_id": "one", "capabilities": ["status", "inspect", "unsupported"]},
                {"host_id": "two", "capabilities": ["inspect", "click"]}
            ]
        });

        assert_eq!(
            native_browser_tool_names_from_status(&status),
            vec![
                "chrome_native_click".to_string(),
                "chrome_native_inspect".to_string(),
                "chrome_native_status".to_string(),
            ]
        );
    }

    #[test]
    fn native_result_is_bound_to_claim_identity_and_expiry() {
        let now = native_unix_time_ms();
        let mut command = NativeCommand {
            id: "claimed-command".to_string(),
            kind: NativeBridgeKind::Browser,
            action: "status".to_string(),
            args: serde_json::json!({}),
            preferred_host_id: None,
            created_at: Instant::now(),
            status: NativeCommandStatus::InFlight,
            host_id: Some("host".to_string()),
            host_instance_id: Some("instance".to_string()),
            host_manifest_id: Some("manifest".to_string()),
            host_manifest_expires_at_ms: Some(now + 1_000),
            claimed_at: Some(Instant::now()),
            result: None,
        };
        assert!(native_result_identity_matches(
            &command, "host", "instance", "manifest", now
        ));
        assert!(!native_result_identity_matches(
            &command, "host", "other", "manifest", now
        ));
        command.host_manifest_expires_at_ms = Some(now);
        assert!(!native_result_identity_matches(
            &command, "host", "instance", "manifest", now
        ));
    }

    #[test]
    fn sorts_conversation_messages_stably() {
        let state = WebState {
            conversations: vec![WebConversation {
                configuration_revision: 1,
                revision: 1,
                is_pinned: false,
                conversation_id: "c1".to_string(),
                title: "Demo".to_string(),
                title_source: ConversationTitleSource::User,
                session_memory: String::new(),
                knowledge_points: String::new(),
                wiki: String::new(),
                last_compacted_at: None,
                last_context_usage: None,
                selected_model_source: "auto".to_string(),
                selected_model_id: String::new(),
                execution_backend: "nola".to_string(),
                external_execution_sessions: Default::default(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            }],
            messages: vec![
                WebMessage {
                    message_id: "m2".to_string(),
                    conversation_id: "c1".to_string(),
                    role: "assistant".to_string(),
                    content: "second".to_string(),
                    include_in_context: true,
                    meta: serde_json::json!({}),
                    created_at: "2026-01-01T00:00:02Z".to_string(),
                },
                WebMessage {
                    message_id: "m1".to_string(),
                    conversation_id: "c1".to_string(),
                    role: "user".to_string(),
                    content: "first".to_string(),
                    include_in_context: true,
                    meta: serde_json::json!({}),
                    created_at: "2026-01-01T00:00:01Z".to_string(),
                },
                WebMessage {
                    message_id: "other".to_string(),
                    conversation_id: "c2".to_string(),
                    role: "user".to_string(),
                    content: "other".to_string(),
                    include_in_context: true,
                    meta: serde_json::json!({}),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                },
            ],
            ..WebState::default()
        };

        let ids = sorted_conversation_messages(&state, "c1")
            .into_iter()
            .map(|message| message.message_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["m1", "m2"]);
    }

    #[test]
    fn web_conversation_round_trip_preserves_execution_unit_sessions() {
        let mut sessions = std::collections::HashMap::new();
        sessions.insert("codex".to_string(), "thread-7".to_string());
        sessions.insert("claude_code".to_string(), "session-42".to_string());
        let conversation = WebConversation {
            configuration_revision: 1,
            revision: 1,
            is_pinned: false,
            conversation_id: "executor-conversation".to_string(),
            title: "External executor".to_string(),
            title_source: ConversationTitleSource::User,
            session_memory: String::new(),
            knowledge_points: String::new(),
            wiki: String::new(),
            last_compacted_at: None,
            last_context_usage: None,
            selected_model_source: "auto".to_string(),
            selected_model_id: String::new(),
            execution_backend: "claude_code".to_string(),
            external_execution_sessions: sessions,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let restored: WebConversation = serde_json::from_value(
            serde_json::to_value(&conversation).expect("encode conversation"),
        )
        .expect("decode conversation");
        assert_eq!(restored.execution_backend, "claude_code");
        assert_eq!(
            restored
                .external_execution_sessions
                .get("claude_code")
                .map(String::as_str),
            Some("session-42")
        );
        assert_eq!(
            restored
                .external_execution_sessions
                .get("codex")
                .map(String::as_str),
            Some("thread-7")
        );
    }

    #[test]
    fn safe_filename_part_removes_unsafe_characters() {
        assert_eq!(safe_filename_part(" A/B:C*D? "), "A-B-C-D");
        assert_eq!(safe_filename_part(""), "chat");
    }

    #[test]
    fn extracts_clean_html_metadata_and_visible_text() {
        let html = r#"
            <html><head>
              <title> Demo &amp; Page </title>
              <meta name="description" content=" Short &lt;summary&gt; ">
              <style>.hidden{display:none}</style>
            </head>
            <body><script>window.location = "/x";</script><h1>Hello</h1><p>World</p></body></html>
        "#;

        assert_eq!(html_title(html).as_deref(), Some("Demo & Page"));
        assert_eq!(
            html_meta_content(html, "description").as_deref(),
            Some("Short")
        );
        let text = clean_html_text(html);
        assert!(text.contains("Hello World"));
        assert!(!text.contains("window.location"));
        assert!(!text.contains(".hidden"));
    }

    #[test]
    fn local_readable_path_requires_allowed_directory() {
        let root = std::env::temp_dir().join(format!("nola-web-test-{}", super::new_id("dir")));
        let allowed = root.join("allowed");
        let outside = root.join("outside");
        fs::create_dir_all(&allowed).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let file = allowed.join("note.txt");
        let outside_file = outside.join("note.txt");
        fs::write(&file, "hello").unwrap();
        fs::write(&outside_file, "nope").unwrap();
        let settings = AgentSettings {
            allowed_dirs: vec![allowed.display().to_string()],
            ..AgentSettings::default()
        };

        assert_eq!(
            resolve_local_readable_path(&file.display().to_string(), &settings, "file").unwrap(),
            file.canonicalize().unwrap()
        );
        assert!(resolve_local_readable_path(
            &outside_file.display().to_string(),
            &settings,
            "file"
        )
        .is_err());
        assert_eq!(read_text_preview(&file, 64_000, 20).unwrap(), "hello");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn script_metadata_and_line_validation_are_conservative() {
        let metadata = build_script_metadata("echo hello", "/tmp", "demo", true).unwrap();
        assert!(metadata.contains("script_automation"));
        assert_eq!(
            validate_script_lines("echo hello", &[String::from("echo")]).unwrap(),
            vec![String::from("echo hello")]
        );
        assert!(validate_script_lines("rm -rf /tmp/x", &[String::from("rm")]).is_err());
        assert!(validate_script_lines("echo hello > out.txt", &[String::from("echo")]).is_err());
        assert!(validate_script_lines("python test.py", &[String::from("echo")]).is_err());
    }

    #[test]
    fn parses_scheduled_and_result_routes() {
        assert_eq!(
            scheduled_action_route("/api/scheduled-actions/a1/run-now"),
            Some(("a1", "run-now"))
        );
        assert_eq!(
            scheduled_action_route("/api/scheduled-actions/a1"),
            Some(("a1", ""))
        );
        assert_eq!(result_detail_route("/api/results/r1"), Some("r1"));
        assert_eq!(result_detail_route("/api/results/r1/extra"), None);
        assert_eq!(
            permission_rule_route("/api/agent/permission_rules/42"),
            Some(42)
        );
        assert_eq!(permission_rule_route("/api/agent/permission_rules/x"), None);
    }

    // Run the actual production TypeScript reducers/components instead of
    // asserting markers in a retired server-rendered HTML implementation.
    fn run_svelte_contract_tests(files: &[&str]) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/frontend");
        let output = std::process::Command::new("node")
            .arg("--test")
            .args(files.iter().map(|file| format!("src/lib/{file}")))
            .current_dir(root)
            .output()
            .expect("Svelte contract tests require Node and web/frontend npm ci");
        assert!(
            output.status.success(),
            "Svelte contract tests failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn svelte_page_routes_are_explicit_and_localized() {
        run_svelte_contract_tests(&["pageRoutes.test.ts", "i18n.test.ts"]);
    }

    #[test]
    fn svelte_workspace_keeps_conversation_navigation_and_locale_behavior() {
        run_svelte_contract_tests(&["conversations.test.ts"]);
    }

    #[test]
    fn web_rejects_retired_cumulative_part_transport() {
        let retired = json!({
            "type": "core_part_updated", "session_id": "conversation",
            "part": {"kind": "text", "text": "must not become an alternate body source"}
        });
        assert!(!web_event_is_core_projection(&retired));
        assert!(!crate::web::mobile_event_is_canonical(&retired));
    }

    #[test]
    fn web_sse_forwards_typed_core_projections_without_trace_wrappers() {
        for event_type in [
            "core_task_updated",
            "core_skill_runtime_updated",
            "core_capabilities_activated",
            "core_permission_projection_updated",
            "core_memory_runtime_projection_updated",
            "assistant_journal_committed_v1",
            "assistant_run_terminal_committed_v1",
            "context_usage",
        ] {
            assert!(crate::web::web_event_is_core_projection(
                &serde_json::json!({"type": event_type})
            ));
            assert!(!crate::web::mobile_event_requires_trace(
                &serde_json::json!({"type": event_type})
            ));
        }
        assert!(!crate::web::web_event_is_core_projection(
            &serde_json::json!({"type": "agent_trace"})
        ));
        assert!(!crate::web::web_event_is_core_projection(
            &serde_json::json!({"type": "tool_result"})
        ));
        assert!(crate::web::mobile_event_is_canonical(
            &serde_json::json!({"type": "context_usage"})
        ));
        assert!(crate::web::mobile_event_is_canonical(&serde_json::json!({
            "type": "agent_trace",
            "trace": {"kind": "tool.completed"}
        })));
        for unsupported_type in [
            "status",
            "iteration",
            "done",
            "tool_result",
            "assistant_delta",
        ] {
            assert!(!crate::web::mobile_event_is_canonical(
                &serde_json::json!({"type": unsupported_type})
            ));
        }
    }

    #[test]
    fn chat_workspace_exposes_inspector_and_core_context_usage() {
        run_svelte_contract_tests(&[
            "chatOverview.test.ts",
            "chatActivity.test.ts",
            "chatSessionSettings.test.ts",
        ]);
    }

    #[test]
    fn completed_turn_requires_sealed_frontier_and_releases_web_composer() {
        run_svelte_contract_tests(&[
            "assistantJournal.test.ts",
            "messageAdmissions.test.ts",
            "assistantTerminal.test.ts",
            "chatTurn.test.ts",
            "chatFlow.test.ts",
            "chatTransport.test.ts",
        ]);
    }

    #[test]
    fn svelte_workspace_uses_actual_lazy_components_and_localized_shell() {
        run_svelte_contract_tests(&["i18n.test.ts", "pageRoutes.test.ts"]);
    }

    #[test]
    fn web_agent_keeps_network_tools_without_local_developer_tools() {
        let settings = AgentSettings::default();
        let schemas = super::web_local_candidate_schemas(&settings);
        let names = schemas
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<std::collections::HashSet<_>>();
        assert!(names.contains("web_search"));
        assert!(names.contains("web_fetch"));
        assert!(names.contains("system_time"));
        assert!(!names.contains("bash"));
        assert!(!names.contains("write_file"));
    }

    #[test]
    fn sse_response_flushes_an_event_before_the_stream_finishes() {
        use std::io::{Read as _, Write as _};

        let server = tiny_http::Server::http("127.0.0.1:0").expect("test server");
        let address = server.server_addr().to_ip().expect("tcp address");
        let (body_tx, body_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let server_thread = std::thread::spawn(move || {
            let request = server.recv().expect("request");
            super::respond_web_response(request, super::sse_response(body_rx)).expect("respond");
        });
        let (line_tx, line_rx) = std::sync::mpsc::channel::<String>();
        let client_thread = std::thread::spawn(move || {
            let mut stream = std::net::TcpStream::connect(address).expect("client connection");
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .expect("read timeout");
            write!(
                stream,
                "GET /stream HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
            )
            .expect("write request");
            stream.flush().expect("flush request");

            let mut received = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = stream.read(&mut buffer).expect("read streamed response");
                if count == 0 {
                    break;
                }
                received.extend_from_slice(&buffer[..count]);
                let response = String::from_utf8_lossy(&received);
                if response.contains("\"text\":\"first\"") {
                    line_tx
                        .send(response.into_owned())
                        .expect("send observed event");
                    break;
                }
            }
            let mut remainder = Vec::new();
            let _ = stream.read_to_end(&mut remainder);
        });

        body_tx
            .send(super::sse_event(
                serde_json::json!({"type":"text","text":"first"}),
            ))
            .expect("send first event");
        let first = line_rx
            .recv_timeout(std::time::Duration::from_millis(500))
            .expect("first SSE event must arrive while producer is still open");
        assert!(first.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(first
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked\r\n"));
        assert!(first.contains("\"text\":\"first\""));
        body_tx
            .send(b"data: [DONE]\n\n".to_vec())
            .expect("send done");
        drop(body_tx);
        client_thread.join().expect("client thread");
        server_thread.join().expect("server thread");
    }

    #[test]
    fn sse_heartbeat_is_a_comment_and_stops_with_its_owner() {
        let (body_tx, body_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let handle =
            super::spawn_sse_heartbeat(body_tx, stop_rx, std::time::Duration::from_millis(5));

        let heartbeat = body_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .expect("heartbeat");
        assert_eq!(heartbeat, b": keepalive\n\n");
        drop(stop_tx);
        handle.join().expect("heartbeat thread");
        assert!(matches!(
            body_rx.recv_timeout(std::time::Duration::from_millis(20)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn parses_weighted_accept_language_without_global_state() {
        assert_eq!(
            parse_accept_language("en-US;q=0.7, zh-CN;q=0.9"),
            Some(crate::i18n::Language::ZhHans)
        );
        assert_eq!(
            parse_accept_language("de-DE,de;q=0.9,en;q=0.8"),
            Some(crate::i18n::Language::De)
        );
        assert_eq!(parse_accept_language("*;q=1"), None);
    }

    #[test]
    fn host_identity_rejects_short_or_unsafe_values() {
        assert!(valid_host_identity("cli-host-safe_123456"));
        assert!(!valid_host_identity("short"));
        assert!(!valid_host_identity("cli-host-with/slash-1234"));
        assert!(!valid_host_identity("cli-host-with\nnewline-1234"));
    }

    #[test]
    fn canonical_remote_host_identity_is_atomic_and_safe() {
        let complete = super::canonical_host_identity_from_request(&json!({
            "submission_id": "turn-42",
            "user_message_id": "user-42",
            "assistant_message_id": "assistant-42"
        }))
        .expect("complete identity");
        assert_eq!(complete.0.as_deref(), Some("turn-42"));
        assert_eq!(complete.1.as_deref(), Some("user-42"));
        assert_eq!(complete.2.as_deref(), Some("assistant-42"));

        assert_eq!(
            super::canonical_host_identity_from_request(&json!({})),
            Err(
                "submission_id, user_message_id and assistant_message_id must be supplied together"
            )
        );

        assert_eq!(
            super::canonical_host_identity_from_request(&json!({
                "submission_id": "turn-42"
            })),
            Err(
                "submission_id, user_message_id and assistant_message_id must be supplied together"
            )
        );
        assert_eq!(
            super::canonical_host_identity_from_request(&json!({
                "submission_id": "turn-42",
                "user_message_id": "user-42\n",
                "assistant_message_id": "assistant-42"
            })),
            Err("canonical identity is invalid")
        );
    }

    #[test]
    fn web_host_access_token_is_persistent_and_private() {
        let root = std::env::temp_dir().join(new_id("nola-web-host-token-test"));
        let path = root.join("mobile-access-token");
        let first = load_or_create_web_host_access_token_at(&path).expect("create token");
        let second = load_or_create_web_host_access_token_at(&path).expect("reload token");
        assert_eq!(first, second);
        assert!(valid_mobile_access_token(&first));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path)
                .expect("token metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn remote_web_host_surface_only_exposes_authenticated_mobile_api() {
        assert!(remote_web_host_route_allowed(&Method::Get, "/api/health"));
        assert!(remote_web_host_route_allowed(
            &Method::Post,
            "/api/mobile/chat/agent/v2/stream"
        ));
        assert!(remote_web_host_route_allowed(
            &Method::Post,
            "/api/mobile/chat/agent/v2/cancel"
        ));
        assert!(remote_web_host_route_allowed(
            &Method::Get,
            "/api/mobile/me"
        ));
        assert!(remote_web_host_route_allowed(&Method::Get, "/login"));
        assert!(remote_web_host_route_allowed(
            &Method::Post,
            "/api/web/login"
        ));
        assert!(remote_web_host_route_allowed(
            &Method::Get,
            "/static/js/main.js"
        ));
        assert!(!remote_web_host_route_allowed(&Method::Post, "/api/health"));
        assert!(!remote_web_host_route_allowed(&Method::Get, "/api/models"));
        assert!(!remote_web_host_route_allowed(
            &Method::Post,
            "/api/chat/agent/v2/stream"
        ));
        assert!(!remote_web_host_route_allowed(
            &Method::Post,
            "/static/js/main.js"
        ));
    }

    #[test]
    fn web_sessions_store_only_digests_and_expire() {
        let now = Instant::now();
        let mut sessions = std::collections::HashMap::new();
        let (raw_session, raw_csrf) = super::insert_web_session(&mut sessions, now);
        assert!(!sessions.contains_key(&raw_session));
        let stored = sessions
            .get(&super::web_secret_digest(&raw_session))
            .expect("session digest should be stored");
        assert_eq!(stored.csrf_digest, super::web_secret_digest(&raw_csrf));
        assert!(stored.expires_at > now);

        sessions.insert(
            "expired".to_string(),
            super::WebBrowserSession {
                csrf_digest: "expired".to_string(),
                created_at: now,
                expires_at: now,
            },
        );
        super::web_cleanup_sessions_locked(&mut sessions, now);
        assert!(!sessions.contains_key("expired"));
    }

    #[test]
    fn mobile_browser_handoff_is_single_use_and_sanitizes_target() {
        let now = Instant::now();
        let mut handoffs = std::collections::HashMap::new();
        let code = super::insert_web_handoff(&mut handoffs, "https://attacker.example/steal", now);

        assert!(!handoffs.contains_key(&code));
        assert_eq!(handoffs.len(), 1);
        assert_eq!(
            handoffs
                .values()
                .next()
                .map(|handoff| handoff.target.as_str()),
            Some("/dashboard")
        );

        let digest = super::web_secret_digest(&code);
        super::web_browser_handoffs().lock().unwrap().insert(
            digest,
            handoffs.remove(&super::web_secret_digest(&code)).unwrap(),
        );
        assert_eq!(
            super::consume_web_handoff(&code).as_deref(),
            Some("/dashboard")
        );
        assert_eq!(super::consume_web_handoff(&code), None);
    }

    #[test]
    fn web_login_redirects_cannot_escape_host() {
        assert_eq!(super::safe_web_next("/models?tab=api"), "/models?tab=api");
        assert_eq!(
            super::safe_web_next("/chat?conversation=1"),
            "/chat?conversation=1"
        );
        assert_eq!(super::safe_web_next("https://example.com"), "/dashboard");
        assert_eq!(super::safe_web_next("//example.com"), "/dashboard");
        assert_eq!(super::safe_web_next("/api/models"), "/dashboard");
        assert_eq!(super::safe_web_next("/login"), "/dashboard");
        for value in [
            "/\\evil.invalid",
            "/x/..//evil.invalid",
            "/chat\n",
            "/api",
            "/chat/../api/models",
            "/x/%2e%2e/login",
        ] {
            assert_eq!(super::safe_web_next(value), "/dashboard", "{value}");
        }
        assert_eq!(super::safe_web_next("/x/../models"), "/models");
        assert_eq!(
            super::safe_web_next("/chat?conversation=opaque%2Fid#latest"),
            "/chat?conversation=opaque%2Fid#latest"
        );
    }

    #[test]
    fn svelte_login_assets_are_public_but_workspace_and_mutations_are_not() {
        assert!(remote_web_host_route_allowed(
            &Method::Get,
            "/ui/assets/index-fixture.js"
        ));
        assert!(remote_web_host_route_allowed(
            &Method::Head,
            "/ui/assets/index-fixture.css"
        ));
        for path in [
            "/ui/index.html",
            "/models",
            "/model-test",
            "/api/model-profiles",
            "/ui/assets/../index.html",
            "/ui/assets/\\private",
        ] {
            assert!(!remote_web_host_route_allowed(&Method::Get, path), "{path}");
        }
        assert!(!remote_web_host_route_allowed(
            &Method::Post,
            "/ui/assets/index-fixture.js"
        ));
        assert!(!remote_web_host_route_allowed(
            &Method::Delete,
            "/ui/assets/index-fixture.js"
        ));
    }

    #[test]
    fn local_model_routes_keep_selection_configuration_and_start_separate() {
        assert_eq!(
            super::local_model_action_route("/api/local-models/qwen3.5-4b-q4km/select"),
            Some(("qwen3.5-4b-q4km", "select"))
        );
        assert_eq!(
            super::local_model_action_route("/api/local-models/qwen3.5-4b-q4km/configuration"),
            Some(("qwen3.5-4b-q4km", "configuration"))
        );
        assert_eq!(
            super::local_model_action_route("/api/local-models/qwen3.5-4b-q4km/start"),
            Some(("qwen3.5-4b-q4km", "start"))
        );
        assert!(super::local_model_action_route("/api/local-models/unknown/start").is_none());
    }

    #[test]
    fn local_and_host_tool_settings_are_independent() {
        let mut settings = AgentSettings::default();
        assert!(settings.host_tools_enabled);
        merge_agent_settings(
            &mut settings,
            serde_json::json!({"enable_local_dev_tools": true}),
        );
        assert!(settings.enable_local_dev_tools);
        assert!(settings.host_tools_enabled);
        merge_agent_settings(
            &mut settings,
            serde_json::json!({"host_tools_enabled": false}),
        );
        assert!(settings.enable_local_dev_tools);
        assert!(!settings.host_tools_enabled);
        merge_agent_settings(
            &mut settings,
            serde_json::json!({"host_tools_enabled": true}),
        );
        assert!(settings.enable_local_dev_tools);
        assert!(settings.host_tools_enabled);
    }

    #[test]
    fn web_permission_profile_defaults_to_cautious_and_persists_strict_values() {
        let mut settings = AgentSettings::default();
        assert_eq!(settings.permission_profile, "cautious");
        assert_eq!(settings.network_policy, "ask");

        for (profile, network_policy) in [
            ("cautious", "ask"),
            ("allow_all", "allow"),
        ] {
            merge_agent_settings(
                &mut settings,
                serde_json::json!({"permission_profile": profile}),
            );
            assert_eq!(settings.permission_profile, profile);
            assert_eq!(settings.network_policy, network_policy);
            assert_eq!(
                super::agent_settings_api(&settings)["permission_profile"],
                profile
            );
        }

        merge_agent_settings(
            &mut settings,
            serde_json::json!({"permission_profile": "moderate"}),
        );
        assert_eq!(settings.permission_profile, "allow_all");
    }

    #[test]
    fn global_web_permission_denial_becomes_core_policy_without_hiding_tools() {
        let rule = WebPermissionRule {
            id: 1,
            tool_name: "web".into(),
            behavior: "deny".into(),
            match_pattern: "*".into(),
        };
        let normalized = super::web_permission_rules_for_core(std::slice::from_ref(&rule));
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].capability_pattern, "web");
        assert_eq!(
            normalized[0].decision,
            nola_core::agent::permissions::PermissionPolicyDecision::Deny
        );

        let tools = super::resolve_web_agent_host_tools_for_turn(
            &AgentSettings::default(),
            std::slice::from_ref(&rule),
        );
        let names = tools
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(names.contains(&"web_search"));
        assert!(names.contains(&"web_fetch"));
        assert!(names.contains(&"read_file"));
    }

    #[test]
    fn parses_persisted_data_source_routes_without_confusing_conversation_routes() {
        assert_eq!(
            super::data_source_route("/api/data-sources/source-1"),
            Some(("source-1", ""))
        );
        assert_eq!(
            super::data_source_route("/api/data-sources/source-1/test"),
            Some(("source-1", "test"))
        );
        assert_eq!(super::data_source_route("/api/data-sources/usage"), None);
        assert_eq!(
            chat_conversation_route("/api/chat/conversations/conversation-1/data-sources/source-1"),
            Some(("conversation-1", "data-sources/source-1"))
        );
    }

    #[test]
    fn svelte_data_source_gallery_executes_actual_editor_and_localization_contract() {
        run_svelte_contract_tests(&["dataSources.test.ts"]);
    }
}

#[cfg(test)]
mod admitted_first_message_title_tests {
    use super::*;
    use nola_application::{ApplicationCommand, BeginAssistantTurn};
    use nola_core::agent::{host::*,AgentCoreConfig,AgentSubmission,AgentItem,ItemId,SessionId};
    use nola_domain::remote_submission::{RemoteSubmissionIdentity,RegisteredSourceSubmission};

    fn setup(path:&Path)->(NolaStore,crate::application_port::CliApplicationPort,RemoteSubmissionIdentity,String){
        let store=NolaStore::open(path).unwrap();
        store.create_web_conversation(&json!(WebConversation::new("main".into(),"New conversation".into(),"2026-11-12T00:00:00Z".into(),"2026-11-12T00:00:00Z".into()))).unwrap();
        let identity=RemoteSubmissionIdentity{source_host_id:"title-test-host".into(),client_id:"title-client".into(),conversation_id:"main".into(),
            submission_id:uuid::Uuid::new_v4().to_string(),user_message_id:"main-user".into(),assistant_message_id:"main-assistant".into()};
        let mut app=crate::application_port::CliApplicationPort::persistent(path).unwrap();
        let request=json!({"conversationId":identity.conversation_id,"expected_host_id":identity.source_host_id,"client_id":identity.client_id,
            "submission_id":identity.submission_id,"user_message_id":identity.user_message_id,"assistant_message_id":identity.assistant_message_id,"messages":[{"role":"user","content":"Actual user input"}]});
        let registration:RegisteredSourceSubmission=app.execute(ApplicationCommand::RegisterSourceSubmission{identity:identity.clone(),request}).unwrap();
        (store,app,identity,registration.receipt.request_digest)
    }
    fn begin(app:&mut crate::application_port::CliApplicationPort,identity:&RemoteSubmissionIdentity,components:Vec<nola_core::context_engine::ContextComponent>)->Result<nola_application::AcceptedAssistantTurn,String>{
        let model=HostCapabilityDescriptor{id:"model.stream".into(),title:"Fixture model".into(),route:HostCapabilityRoute::Model,
            availability:CapabilityAvailability::Available,exposure:HostCapabilityExposure::Internal,permission_domain:None,input_schema:json!({"type":"object"}),input_normalization:Default::default(),
            output_policy:HostOutputPolicy{preview_char_limit:1000,persist_full_output:true,allow_artifacts:false},supports_streaming:true,supports_argument_streaming:false,
            parallelism:HostCapabilityParallelism::Serial,cancellation:HostCancellationMode::Cooperative,replay_safety:HostReplaySafety::Recoverable};
        let config=AgentCoreConfig{session_id:Some(SessionId::from_string(identity.conversation_id.clone())),model:Some("fixture".into()),turn_identity:crate::test_model_identity::identity(),
            host_catalog:Some(HostCapabilityCatalog{host_id:HostId("title-test-host".into()),revision:1,availability_epoch:1,lease:None,capabilities:[("model.stream".into(),model)].into_iter().collect()}),..Default::default()};
        let existing=app.load_conversation(&identity.conversation_id)?.unwrap();
        app.begin_assistant_turn(BeginAssistantTurn{conversation_id:identity.conversation_id.clone(),expected_configuration_revision:Some(existing.configuration_revision),expected_parent_message_id:None,
            conversation_title:existing.title,user_message_id:identity.user_message_id.clone(),user_part_id:"main-user:text".into(),user_text:"Actual user input".into(),user_content_json:None,
            assistant_message_id:identity.assistant_message_id.clone(),context_checkpoint_id:None,core_config:config,created_at_ms:10,
            submission:AgentSubmission::UserInput{items:vec![AgentItem::user_message(ItemId::from_string(identity.user_message_id.clone()),"Actual user input")],
                turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:components,submission_id:Some(identity.submission_id.clone()),submitted_at_epoch_ms:Some(10),delivery:nola_core::agent::AgentInputDelivery::CurrentTurn}}).map_err(|error|error.to_string())
    }
    #[test]
    fn prepared_or_wrong_epoch_source_cannot_adopt_a_title(){
        let root=tempfile::tempdir().unwrap();let(store,mut app,id,digest)=setup(&root.path().join("rejected.db"));
        assert!(!adopt_first_message_conversation_title_at(&store,&id,&digest,"Stale draft must not rename").unwrap());
        let component=nola_core::context_engine::ContextComponent{kind:nola_core::context_engine::ContextComponentKind::MainExpertSelection,
            id:nola_domain::expert_task_target::MAIN_EXPERT_SELECTION_COMPONENT_ID.into(),source:None,
            content:json!({"version":1,"expected_source_store_epoch":"retired-store","selected_experts":[{"agent_id":"reviewer","agent_revision":1}]})};
        assert!(begin(&mut app,&id,vec![component]).unwrap_err().contains("main_expert_selection_source_changed"));
        assert!(!adopt_first_message_conversation_title_at(&store,&id,&digest,"Stale draft must not rename").unwrap());
        let view=store.get_web_conversation("main").unwrap().unwrap();assert_eq!(view["title"],"New conversation");assert_eq!(view["titleSource"],"automatic");
        assert!(app.load_conversation("main").unwrap().unwrap().messages.is_empty());
    }
    #[test]
    fn admitted_provider_failure_still_adopts_title_and_preserves_a_later_user_title(){
        let root=tempfile::tempdir().unwrap();let(store,mut app,id,digest)=setup(&root.path().join("admitted.db"));
        let admitted=begin(&mut app,&id,vec![]).unwrap();
        let requests=app.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests{run_id:id.submission_id.clone(),worker_generation:"title-fixture".into(),lease_owner:"title-fixture".into(),claimed_at_ms:11,lease_expires_at_ms:1000,limit:20}).unwrap();
        let request=requests.requests.into_iter().find(|request|matches!(request.payload,HostRequest::ModelStream{..})).unwrap();
        for (sequence,payload) in [(1,HostResponse::ExecutionStarted),(2,HostResponse::Failed{error:HostError{code:"provider_unavailable".into(),message:"Fixture provider could not connect".into(),retryable:false,http_status:Some(503)}})] {
            app.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse{conversation_id:id.conversation_id.clone(),run_id:id.submission_id.clone(),invocation_config_digest:admitted.invocation_config_digest.clone(),received_at_ms:12+sequence as i64,
                response:HostResponseEnvelope{request_id:request.request_id.clone(),attempt_id:request.attempt_id.clone(),effect_id:request.effect_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),
                    session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence,payload}}).unwrap();
        }
        assert!(adopt_first_message_conversation_title_at(&store,&id,&digest,"Actual user input").unwrap());
        let view=store.get_web_conversation("main").unwrap().unwrap();assert_eq!(view["title"],"Actual user input");assert_eq!(view["titleSource"],"first_message");
        let mut wrong=id.clone();wrong.client_id="different-client".into();assert!(adopt_first_message_conversation_title_at(&store,&wrong,&digest,"Foreign title").is_err());
        store.rename_web_conversation("main","My chosen title",crate::store::ConversationTitleSource::User,view["revision"].as_i64().unwrap()).unwrap();
        assert!(!adopt_first_message_conversation_title_at(&store,&id,&digest,"Do not overwrite my title").unwrap());
        assert_eq!(store.get_web_conversation("main").unwrap().unwrap()["title"],"My chosen title");
    }
}
