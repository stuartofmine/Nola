// SPDX-License-Identifier: Apache-2.0
//! One bounded historical interpretation feeds presentation and model context.
use nola_domain::{CanonicalPartOwner, HistoricalPartProjection, SemanticValue, StructuredValue};
use nola_store_port::conversation_history::{StoredConversation, StoredMessage, StoredPart};
use nola_store_port::historical_part_projection::HistoricalPartProjectionRepository;

pub(crate) fn read<S: HistoricalPartProjectionRepository + ?Sized>(
    store: &S,
    conversation: &StoredConversation,
    message: &StoredMessage,
    part: &StoredPart,
    remaining: &mut usize,
) -> Result<Option<HistoricalPartProjection>, String> {
    let owner =
        CanonicalPartOwner::new(conversation.id.clone(), message.id.clone(), part.id.clone())
            .map_err(|error| error.to_string())?;
    match store
        .read_historical_part_with_budget(&owner, conversation.revision, (*remaining).min(8192))
        .map_err(|error| error.to_string())?
    {
        nola_domain::HistoricalPartRead::Absent => Ok(None),
        nola_domain::HistoricalPartRead::Available(projection) => {
            let bytes = serde_json::to_vec(&projection)
                .map_err(|error| error.to_string())?
                .len();
            *remaining = remaining
                .checked_sub(bytes)
                .ok_or("historical projection exceeds selection budget")?;
            Ok(Some(*projection))
        }
        nola_domain::HistoricalPartRead::BudgetDeferred { canonical_kind } => Ok(Some(
            not_loaded_history(owner, conversation.revision, canonical_kind),
        )),
    }
}

pub(crate) fn not_loaded_history(
    owner: CanonicalPartOwner,
    revision: i64,
    canonical_kind: String,
) -> HistoricalPartProjection {
    HistoricalPartProjection {
        owner,
        canonical_revision: revision,
        canonical_kind,
        source: None,
        schema: None,
        binding: None,
        status: nola_domain::HistoricalProjectionStatus::NotLoaded {
            reason: nola_domain::HistoricalNotLoadedReason::SelectionBudget,
        },
        fields: std::collections::BTreeMap::new(),
        canonical_text: None,
    }
}

/// Every context item uses a separate identity domain from canonical Part IDs.
/// A canonical Part named `p:call` cannot alias a tool item generated from `p`.
pub(crate) fn context_item_id(
    conversation: &str,
    message: &str,
    part: Option<&str>,
    role: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let key = serde_json::to_vec(&(conversation, message, part, role))
        .expect("string tuple serialization");
    format!("context-item:{:x}", Sha256::digest(key))
}

pub(crate) fn value<'a>(
    part: &'a HistoricalPartProjection,
    name: &str,
) -> Option<&'a StructuredValue> {
    match part.fields.get(name) {
        Some(SemanticValue::Complete { value }) => Some(value),
        _ => None,
    }
}
pub(crate) fn text<'a>(part: &'a HistoricalPartProjection, name: &str) -> Option<&'a str> {
    match value(part, name) {
        Some(StructuredValue::String(text)) => Some(text),
        _ => None,
    }
}

pub(crate) fn unsigned(part: &HistoricalPartProjection, name: &str) -> Option<u64> {
    match value(part, name) {
        Some(StructuredValue::Number(value)) => value.as_u64(),
        _ => None,
    }
}

pub(crate) fn string_array(part: &HistoricalPartProjection, name: &str) -> Option<Vec<String>> {
    match value(part, name) {
        Some(StructuredValue::Array(values)) => values
            .iter()
            .map(|value| match value {
                StructuredValue::String(value) if !value.is_empty() => Some(value.clone()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

pub(crate) fn sha256<'a>(part: &'a HistoricalPartProjection, name: &str) -> Option<&'a str> {
    text(part, name).filter(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(test)]
mod tests {
    use nola_agent_engine::agent::{
        context_window::compaction_source_fingerprint, AgentItem, AgentRole, ItemId,
    };
    use nola_store::{
        AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput, NolaStore,
    };

    fn store(raw: serde_json::Value, kind: &str) -> NolaStore {
        let mut store = NolaStore::open(":memory:").unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "c".into(),
                title: "History".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "c".into(),
                expected_message_count: 0,
                updated_at_ms: 2,
                messages: vec![MessageInput {
                    id: "m".into(),
                    run_id: Some("r".into()),
                    role: "assistant".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![MessagePartInput {
                        id: "p".into(),
                        kind: kind.into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: String::new(),
                        content_json: Some(raw.to_string()),
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                }],
            })
            .unwrap();
        store
    }
    fn tool(input: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"kind":"tool","id":"p","message_id":"m","turn_id":"r",
            "call_id":"old-call","capability_id":"file.read","title":"Read","state":"completed",
            "input":input,"structured_output":{"ok":true},"artifacts":[]})
    }
    #[test]
    fn visible_tool_json_without_original_source_never_fabricates_a_tool_pair() {
        for input in [
            serde_json::json!({"path":"original"}),
            serde_json::json!({"value":"x".repeat(12000)}),
        ] {
            let store = store(tool(input.clone()), "tool");
            let conversation = store.load_conversation("c").unwrap().unwrap();
            let (items, owners, _) =
                crate::durable_conversation_items(&store, &conversation, None).unwrap();
            assert_eq!(items.len(), 1);
            let AgentItem::Message { content, .. } = &items[0] else {
                panic!("unbound display tool was promoted to an original tool item")
            };
            assert!(content.contains("omitted"));
            assert!(owners.values().all(|owner| owner.message_id == "m"));
        }
    }
    #[test]
    fn message_part_view_text_is_restored_from_the_same_typed_reader() {
        let store = store(
            serde_json::json!({"kind":"text","part_id":"p","revision":1,
            "markdown":"original 中😀","is_final":true}),
            "text",
        );
        let conversation = store.load_conversation("c").unwrap().unwrap();
        let (items, _, _) = crate::durable_conversation_items(&store, &conversation, None).unwrap();
        assert!(matches!(&items[0], AgentItem::Message { id, content, .. }
            if id.as_str()==super::context_item_id("c", "m", Some("p"), "part") && content=="original 中😀"));
    }

    #[test]
    fn cold_context_restores_compaction_lineage_to_exact_canonical_message_position() {
        let canonical_source = AgentItem::Message {
            id: ItemId::from_string("source-item".into()),
            role: AgentRole::User,
            content: "exact canonical original 中😀".into(),
        };
        let fingerprint = compaction_source_fingerprint(&[canonical_source]).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("context.db");
        let mut store = NolaStore::open(&database).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "c".into(),
                title: "History".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "c".into(),
                expected_message_count: 0,
                updated_at_ms: 3,
                messages: vec![
                    MessageInput {
                        id: "source-message".into(),
                        run_id: Some("source-turn".into()),
                        role: "user".into(),
                        status: "completed".into(),
                        ordinal: 1,
                        created_at_ms: 1,
                        completed_at_ms: Some(1),
                        parts: vec![MessagePartInput {
                            id: "source-item".into(),
                            kind: "text".into(),
                            ordinal: 1,
                            state: "completed".into(),
                            content_text: "exact canonical original 中😀".into(),
                            content_json: Some(
                                serde_json::json!({"kind":"text","id":"source-item","message_id":"source-message","turn_id":"source-turn","text":"exact canonical original 中😀","is_complete":true}).to_string(),
                            ),
                            content_digest: None,
                            created_at_ms: 1,
                            updated_at_ms: 1,
                        }],
                    },
                    MessageInput {
                        id: "summary-message".into(),
                        run_id: Some("summary-turn".into()),
                        role: "assistant".into(),
                        status: "completed".into(),
                        ordinal: 2,
                        created_at_ms: 2,
                        completed_at_ms: Some(2),
                        parts: vec![MessagePartInput {
                            id: "summary-item".into(),
                            kind: "compaction".into(),
                            ordinal: 1,
                            state: "completed".into(),
                            content_text: "checkpoint".into(),
                            content_json: Some(
                                serde_json::json!({"kind":"compaction","id":"summary-item","message_id":"summary-message","turn_id":"summary-turn","summary":"checkpoint","retained_item_ids":["source-item"],"source_fingerprint":fingerprint,"original_token_count":77,"compacted_token_count":4}).to_string(),
                            ),
                            content_digest: None,
                            created_at_ms: 2,
                            updated_at_ms: 2,
                        }],
                    },
                ],
            })
            .unwrap();

        drop(store);
        let store = NolaStore::open(&database).unwrap();
        let conversation = store.load_conversation("c").unwrap().unwrap();
        let (items, _, _) = crate::durable_conversation_items(&store, &conversation, None).unwrap();
        let compacted = items
            .iter()
            .find_map(|item| match item {
                AgentItem::Compaction {
                    source_item_ids,
                    source_fingerprint,
                    original_token_count,
                    ..
                } => Some((source_item_ids, source_fingerprint, original_token_count)),
                _ => None,
            })
            .expect("cold reconstruction retains the compaction checkpoint");
        assert_eq!(
            compacted.0.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            ["source-item"]
        );
        assert_eq!(compacted.1, &fingerprint);
        assert_eq!(*compacted.2, 77);

        let source_id = compacted.0[0].as_str();
        let (message_position, source_part) = conversation
            .messages
            .iter()
            .enumerate()
            .find_map(|(position, message)| {
                message
                    .parts
                    .iter()
                    .find(|part| part.id == source_id)
                    .map(|part| (position, part))
            })
            .expect("retained source ID locates the canonical source Part");
        assert_eq!(message_position, 0);
        assert_eq!(conversation.messages[message_position].id, "source-message");
        assert_eq!(source_part.content_text, "exact canonical original 中😀");
    }
}

/// Charge complete AgentItems and ownership metadata, including omission notes.
/// Drop whole Message groups so a tool call/result pair is never split.
pub(crate) fn bound_context(
    conversation_id: &str,
    items: &mut Vec<nola_agent_engine::agent::AgentItem>,
    owners: &mut std::collections::BTreeMap<
        String,
        nola_agent_engine::agent::AgentItemOwnership,
    >,
    maximum_bytes: usize,
) -> Result<(), String> {
    use nola_agent_engine::agent::{AgentItem, AgentRole, ItemId};
    let size = |items: &Vec<AgentItem>,
                owners: &std::collections::BTreeMap<
        String,
        nola_agent_engine::agent::AgentItemOwnership,
    >| {
        serde_json::to_vec(&(items, owners))
            .map(|bytes| bytes.len())
            .map_err(|error| error.to_string())
    };
    let mut seen = std::collections::BTreeSet::new();
    for item in items.iter() {
        if !seen.insert(item.id().as_str()) || !owners.contains_key(item.id().as_str()) {
            return Err("context item identity is duplicated or ownership is missing".into());
        }
    }
    if seen.len() != owners.len() {
        return Err("context ownership has no matching item".into());
    }
    let mut omitted = 0;
    while size(items, owners)? > maximum_bytes {
        let first = items
            .first()
            .ok_or("context metadata exceeds byte budget")?;
        let first_owner = owners
            .get(first.id().as_str())
            .ok_or("context item ownership is missing")?
            .clone();
        let last = items.last().unwrap();
        let last_owner = owners
            .get(last.id().as_str())
            .ok_or("context item ownership is missing")?
            .clone();
        if first_owner.message_id == last_owner.message_id {
            items.clear();
            owners.clear();
            let id = context_item_id(conversation_id, &last_owner.message_id, None, "budget");
            items.push(AgentItem::Message { id: ItemId::from_string(id.clone()), role: AgentRole::System,
                content: format!("Historical message {} in conversation {conversation_id} exceeded the context byte budget. Its original content is available through conversation recall; no partial tool call or result was restored.", last_owner.message_id) });
            owners.insert(id, last_owner);
            if size(items, owners)? > maximum_bytes {
                return Err("context omission metadata exceeds budget".into());
            }
            return Ok(());
        }
        items.retain(|item| {
            let remove = owners
                .get(item.id().as_str())
                .is_some_and(|owner| owner.message_id == first_owner.message_id);
            if remove {
                owners.remove(item.id().as_str());
                omitted += 1;
            }
            !remove
        });
    }
    if omitted != 0 {
        let owner = owners
            .get(
                items
                    .first()
                    .ok_or("empty context selection")?
                    .id()
                    .as_str(),
            )
            .ok_or("context ownership unavailable")?
            .clone();
        let id = context_item_id(conversation_id, &owner.message_id, None, "budget-notice");
        items.insert(0, AgentItem::Message { id: ItemId::from_string(id.clone()), role: AgentRole::System,
            content: format!("{omitted} historical items were omitted to keep the context within its encoded byte budget. Original messages remain available through conversation recall.") });
        owners.insert(id, owner);
        // A notice is charged too. At most one further whole-group pass is
        // ordinarily needed; recursive passes strictly reduce original groups.
        if size(items, owners)? > maximum_bytes {
            bound_context(conversation_id, items, owners, maximum_bytes)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod budget_tests {
    use nola_agent_engine::agent::{
        AgentItem, AgentItemOwnership, AgentRole, CallId, ItemId,
    };
    use std::collections::BTreeMap;
    #[test]
    fn encoded_budget_drops_whole_tool_pairs_and_charges_notice_and_ownership() {
        let mut items = Vec::new();
        let mut owners = BTreeMap::new();
        for (message, size) in [("old", 6000), ("new", 8)] {
            let call = format!("{message}:call");
            let result = format!("{message}:result");
            items.push(AgentItem::ToolCall {
                id: ItemId::from_string(call.clone()),
                call_id: CallId::from_string(message.into()),
                name: "file.read".into(),
                arguments: serde_json::json!({"value":"x".repeat(size)}),
            });
            items.push(AgentItem::ToolResult {
                id: ItemId::from_string(result.clone()),
                call_id: CallId::from_string(message.into()),
                name: "file.read".into(),
                output: serde_json::json!({"ok":true}),
                is_error: false,
                full_output_ref: None,
                artifacts: vec![],
            });
            for id in [call, result] {
                owners.insert(
                    id,
                    AgentItemOwnership {
                        source: Default::default(),
                        turn_id: "r".into(),
                        message_id: message.into(),
                    },
                );
            }
        }
        super::bound_context("c", &mut items, &mut owners, 3000).unwrap();
        assert!(serde_json::to_vec(&(&items, &owners)).unwrap().len() <= 3000);
        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(item, AgentItem::ToolCall { .. }))
                .count(),
            1
        );
        assert_eq!(
            items
                .iter()
                .filter(|item| matches!(item, AgentItem::ToolResult { .. }))
                .count(),
            1
        );
        assert!(owners.values().all(|owner| owner.message_id == "new"));
        assert!(
            matches!(&items[0],AgentItem::Message{role:AgentRole::System,content,..} if content.contains("omitted"))
        );
    }
}
