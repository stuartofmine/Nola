// SPDX-License-Identifier: Apache-2.0
//! Saved, conversation-owned documents for a new interactive turn.
use crate::application_port::CliApplicationPort;
use nola_application::ApplicationCommand;
use nola_core::context_engine::{ContextComponent, ContextComponentKind, ContextComponentSource, ContextPlatform};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    id: String,
    revision: i64,
    configuration_revision: i64,
    settings_json: Option<String>,
}

pub(crate) fn load(app: &mut CliApplicationPort, conversation_id: &str) -> Result<Vec<ContextComponent>, String> {
    let metadata: Option<Metadata> = app.execute(ApplicationCommand::LoadConversationMetadata {
        conversation_id: conversation_id.into(),
    })?;
    let Some(metadata) = metadata else { return Ok(Vec::new()) };
    if metadata.id != conversation_id || metadata.revision < 1 {
        return Err("invalid knowledge metadata ownership".into());
    }
    components(metadata.settings_json.as_deref())
}

fn components(raw: Option<&str>) -> Result<Vec<ContextComponent>, String> {
    let Some(raw) = raw else { return Ok(Vec::new()) };
    let settings: Value = serde_json::from_str(raw).map_err(|_| "invalid knowledge settings JSON")?;
    let fields = settings.as_object().ok_or("knowledge settings must be an object")?;
    let Some(knowledge) = fields.get("knowledge").filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let knowledge = knowledge.as_object().ok_or("knowledge must be an object")?;
    if knowledge.get("updatedAt").is_some_and(|value| !value.is_null() && !value.is_number()) {
        return Err("invalid knowledge timestamp".into());
    }
    let mut result = Vec::new();
    for (field, id, title) in [
        ("keyPointsMarkdown", "conversation-knowledge-points", "Conversation knowledge points"),
        ("wikiMarkdown", "conversation-knowledge-wiki", "Conversation Wiki"),
    ] {
        let text = knowledge.get(field).and_then(Value::as_str).ok_or("knowledge Markdown must be a string")?;
        if text.trim().is_empty() { continue; }
        result.push(ContextComponent {
            kind: ContextComponentKind::Document,
            id: id.into(),
            content: Value::String(text.into()),
            source: Some(ContextComponentSource {
                platform: Some(ContextPlatform::Shared),
                title: Some(title.into()),
                mime_type: Some("text/markdown".into()),
                ..ContextComponentSource::default()
            }),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn authored_knowledge_is_not_cross_session_memory_and_preserves_markdown() {
        let raw = json!({"knowledge":{"keyPointsMarkdown":"  事实\n", "wikiMarkdown":"# Wiki\n"},
            "conversation":{"knowledgePoints":"obsolete", "wiki":"obsolete"}}).to_string();
        let result = components(Some(&raw)).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|item| item.kind == ContextComponentKind::Document));
        assert_eq!(result[0].content, "  事实\n");
        assert_eq!(result[1].content, "# Wiki\n");
        let admitted = nola_core::context_engine::admit_context_components(result, &nola_core::memory::MemoryPolicy {
            long_term_enabled: false, ..Default::default()
        });
        assert_eq!(admitted.len(), 2);
        let prepared = nola_core::context_engine::prepare_context(&nola_core::context_engine::ContextPrepareInput {
            platform: ContextPlatform::Web,
            components: admitted,
            messages: vec![json!({"role":"user", "content":"Use this conversation's notes"})],
            budget: nola_core::context_engine::ContextBudgetSpec {
                context_window_tokens: Some(128_000), reserved_output_tokens: 4096,
                ..Default::default()
            },
            ..Default::default()
        });
        let prompt = serde_json::to_string(&prepared.messages).unwrap();
        assert!(prompt.contains("事实"));
        assert!(prompt.contains("# Wiki"));
        assert!(!prompt.contains("obsolete"));
    }

    #[test]
    fn absence_is_empty_but_malformed_documents_are_not_silently_dropped() {
        for value in [json!({}), json!({"knowledge":null}), json!({"conversation":{"knowledgePoints":"old"}}),
            json!({"knowledge":{"keyPointsMarkdown":" \n", "wikiMarkdown":""}})] {
            assert!(components(Some(&value.to_string())).unwrap().is_empty());
        }
        for knowledge in [json!("wrong"), json!({}), json!({"keyPointsMarkdown":"facts"}),
            json!({"keyPointsMarkdown":2,"wikiMarkdown":""}),
            json!({"keyPointsMarkdown":"facts","wikiMarkdown":"wiki","updatedAt":"yesterday"})] {
            assert!(components(Some(&json!({"knowledge":knowledge}).to_string())).is_err());
        }
    }

    #[test]
    fn saved_configuration_is_read_only_and_isolated_by_conversation() {
        let mut app = CliApplicationPort::ephemeral().unwrap();
        let create: ApplicationCommand = serde_json::from_value(json!({"type":"create_conversation", "input":{
            "id":"one", "title":"One", "settings_json":null, "created_at_ms":1,"updated_at_ms":1
        }})).unwrap();
        let _: Value = app.execute(create).unwrap();
        let _: nola_domain::conversation_configuration::ConfigurationCommitResult = app.execute(ApplicationCommand::PatchConversationConfiguration {
            input: nola_application::conversation_configuration::PatchConversationConfigurationInput {
                conversation_id:"one".into(), expected_revision:1, updated_at_ms:2,
                patch_json:json!({"knowledge":{"wikiMarkdown":"Saved from Web"}}).to_string().parse().unwrap(),
            }
        }).unwrap();
        assert_eq!(load(&mut app, "one").unwrap()[0].content, "Saved from Web");
        assert!(load(&mut app, "other").unwrap().is_empty());
        let metadata: Metadata = app.execute::<Option<Metadata>>(ApplicationCommand::LoadConversationMetadata {
            conversation_id:"one".into()
        }).unwrap().unwrap();
        assert_eq!(metadata.revision, 2);
    }
}
