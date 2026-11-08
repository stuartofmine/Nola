// SPDX-License-Identifier: Apache-2.0
//! Resolve historical references only among validated, selected canonical Parts.
//! Evidence remains model data; it neither restores a Run nor grants URI access.
use crate::historical_part_projection::{text, value};
use nola_domain::{
    HistoricalPartProjection, HistoricalPartSchema, HistoricalProjectionStatus,
    SemanticIdentityAssessment,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn supported(part: &HistoricalPartProjection) -> bool {
    part.status == HistoricalProjectionStatus::Supported
        && matches!(
            part.binding,
            Some(
                SemanticIdentityAssessment::Canonical
                    | SemanticIdentityAssessment::VerifiedLocalDerivation
            )
        )
}
fn mapped_id(part: &HistoricalPartProjection, kind: &str, original: &str) -> String {
    mapped_source_id(&part.owner,kind,original)
}
pub(crate) fn mapped_source_id(owner: &nola_domain::CanonicalPartOwner, kind: &str, original: &str) -> String {
    let namespace = serde_json::to_vec(&(
        owner.conversation_id(),
        owner.message_id(),
        owner.part_id(),
        original,
    ))
    .expect("string tuple serialization");
    format!("{kind}:{:x}", Sha256::digest(namespace))
}
// Calls and evidence are scoped to the selected target message and validated
// original ownership. Equal call strings in another message/schema cannot bind.
fn key(part: &HistoricalPartProjection, original: &str) -> Option<String> {
    if !supported(part) {
        return None;
    }
    let (schema, message, turn) = match part.schema? {
        HistoricalPartSchema::AgentPart => (
            "agent_part",
            text(part, "message_id")?,
            text(part, "turn_id")?,
        ),
        HistoricalPartSchema::MessagePartView => ("message_part_view", part.owner.message_id(), ""),
    };
    Some(
        serde_json::to_string(&(
            part.owner.conversation_id(),
            part.owner.message_id(),
            schema,
            message,
            turn,
            original,
        ))
        .expect("string tuple serialization"),
    )
}
#[derive(Default)]
pub(crate) struct HistoricalReferenceIndex {
    calls: BTreeMap<String, Option<String>>,
    sources: BTreeMap<String, Option<(String, String)>>,
}
fn insert_unique<T>(map: &mut BTreeMap<String, Option<T>>, key: String, value: T) {
    use std::collections::btree_map::Entry;
    match map.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(Some(value));
        }
        Entry::Occupied(mut entry) => {
            entry.insert(None);
        }
    }
}
impl HistoricalReferenceIndex {
    pub fn new<'a>(parts: impl Iterator<Item = &'a HistoricalPartProjection>,
        hydrated_tools: &BTreeMap<String,Vec<nola_agent_engine::agent::protocol::AgentItem>>) -> Self {
        let mut index = Self::default();
        for part in parts {
            if let Some(original_call) = (part.canonical_kind=="tool" && supported(part))
                .then(||text(part,if part.schema==Some(HistoricalPartSchema::AgentPart) {"call_id"}else{"tool_call_id"})).flatten() {
                let mapped=mapped_id(part,"history",original_call);
                let capability=text(part,"capability_id");
                let exact_pair=hydrated_tools.get(part.owner.part_id()).is_some_and(|items| {
                    use nola_agent_engine::agent::protocol::AgentItem;
                    matches!(items.as_slice(),[AgentItem::ToolCall { call_id,name,.. },AgentItem::ToolResult { call_id:result_call,name:result_name,.. }]
                        if call_id.as_str()==mapped && result_call==call_id && Some(name.as_str())==capability && result_name==name)
                });
                if exact_pair {
                    if let Some(key) = key(part, original_call) {
                        insert_unique(&mut index.calls, key, mapped);
                    }
                }
            }
            if part.canonical_kind == "source" && text(part, "title").is_some() {
                if let (Some(source), Some(uri)) = (text(part, "source_id"), text(part, "uri")) {
                    if let Some(key) = key(part, source) {
                        insert_unique(
                            &mut index.sources,
                            key,
                            (mapped_id(part, "history-source", source), uri.to_owned()),
                        );
                    }
                }
            }
        }
        index
    }
    pub fn call(&self, part: &HistoricalPartProjection, original: &str) -> Option<&str> {
        self.calls.get(&key(part, original)?)?.as_deref()
    }
    /// Produce a JSON data record, never an instruction or an executable attachment.
    pub fn evidence(
        &self,
        part: &HistoricalPartProjection,
    ) -> Result<Option<String>, &'static str> {
        if !supported(part) {
            return Ok(None);
        }
        let kind = part.canonical_kind.as_str();
        if matches!(kind, "attachment" | "artifact" | "task" | "step") {
            let mut record = serde_json::json!({"kind":format!("historical_{kind}"),
                "part_id":part.owner.part_id(), "message_id":part.owner.message_id()});
            // IDs from the original task/attachment are descriptive source data,
            // not a target Run or a new executable attachment identity.
            for field in [
                "artifact",
                "name",
                "uri",
                "mime_type",
                "title",
                "status",
                "display_key",
                "state",
                "progress",
                "current_step",
                "result_ref",
                "cancel_requested",
            ] {
                if let Some(value) = value(part, field) {
                    record[field] =
                        serde_json::to_value(value).expect("validated structured value");
                }
            }
            return Ok(Some(record.to_string()));
        }
        if !matches!(kind, "source" | "citation") {
            return Ok(None);
        }
        let source = text(part, "source_id").ok_or("incomplete_source_id")?;
        let uri = text(part, "uri").ok_or("incomplete_source_uri")?;
        let title = text(part, "title").ok_or("incomplete_source_title")?;
        let source_key = key(part, source).ok_or("unverified_reference_scope")?;
        let (source_id, source_uri) = self
            .sources
            .get(&source_key)
            .and_then(Option::as_ref)
            .ok_or("missing_or_ambiguous_source")?;
        if source_uri != uri {
            return Err("source_uri_mismatch");
        }
        let mut record = serde_json::json!({"kind":format!("historical_{kind}"),
            "part_id":part.owner.part_id(), "message_id":part.owner.message_id(),
            "source_id":source_id, "title":title, "uri":uri});
        if kind == "citation" {
            let old_call =
                text(part, "origin_tool_call_id").ok_or("incomplete_origin_tool_call")?;
            let call = self
                .call(part, old_call)
                .ok_or("missing_or_ambiguous_origin_tool_call")?;
            record["origin_tool_call_id"] = call.into();
            record["citation_id"] = mapped_id(
                part,
                "history-citation",
                text(part, "citation_id").unwrap_or(part.owner.part_id()),
            )
            .into();
        }
        for field in [
            "mime_type",
            "evidence",
            "snippet",
            "location",
            "range",
            "page",
            "line_start",
            "line_end",
            "range_start",
            "range_end",
        ] {
            if let Some(value) = value(part, field) {
                record[field] = serde_json::to_value(value).expect("validated structured value");
            }
        }
        Ok(Some(record.to_string()))
    }
}

#[cfg(test)]
use nola_store::NolaStore;
#[cfg(test)]
#[path = "../../nola-store/tests/support/terminal_sources.rs"]
mod terminal_sources;

#[cfg(test)]
mod tests {
    use nola_domain::{AssistantTurnBinding, CanonicalBranchIntent, CanonicalBranchTarget, PartMutation};
    use nola_agent_engine::agent::{AgentItem, AgentRole, CallId, ItemId};
    use nola_store::{
        AppendMessagesInput, BeginTurnInput, MessageInput, MessagePartInput, NolaStore,
    };
    use nola_store_port::assistant_journal::{AssistantJournalRepository, AssistantSourceCommitStamp};
    use nola_store_port::assistant_ownership::AssistantJournalPlanningRepository;
    use nola_store_port::run_effect::{CheckpointWrite, CommitTransitionInput};
    use nola_store_port::tool_source::{ToolSourceInput, ToolSourceKind};
    use serde_json::{json, Value};

    struct OriginalTool {
        part: String,
        call: String,
        arguments: Value,
        result_present: bool,
    }
    fn original_tool(part: &str, call: &str, arguments: Value) -> OriginalTool {
        OriginalTool {part:part.into(),call:call.into(),arguments,result_present:true}
    }
    fn originals() -> Vec<OriginalTool> {
        vec![original_tool("tool","old-call",json!({"path":"original"}))]
    }

    fn parts(view: bool) -> Vec<Value> {
        let mut parts = vec![
            json!({"kind":"citation","id":"cite","message_id":"m","turn_id":"r",
                "citation_id":"old-citation","source_id":"old-source","title":"Source","uri":"file://original",
                "origin_tool_call_id":"old-call", "range":{"start":2,"end":8}}),
            json!({"kind":"source","id":"source","message_id":"m","turn_id":"r",
                "source_id":"old-source","title":"Source","uri":"file://original"}),
            json!({"kind":"tool","id":"tool","message_id":"m","turn_id":"r",
                "execution_source":{"kind":"model"},
                "call_id":"old-call","capability_id":"file.read","title":"Read","state":"completed",
                "input":{"path":"original"},"structured_output":{"ok":true},"artifacts":[]}),
        ];
        if view {
            for part in &mut parts {
                let object = part.as_object_mut().unwrap();
                let id = object.remove("id").unwrap();
                object.remove("message_id");
                object.remove("turn_id");
                object.insert("part_id".into(), id);
                object.insert("revision".into(), 1.into());
                match object["kind"].as_str().unwrap() {
                    "tool" => {
                        let call = object.remove("call_id").unwrap();
                        let output = object.remove("structured_output").unwrap();
                        object.insert("tool_call_id".into(), call);
                        object.insert("output".into(), output);
                        object.insert("origin".into(), json!({"kind":"builtin"}));
                        object.insert("affected_resources".into(), json!([]));
                    }
                    "citation" => {
                        object.remove("range");
                        object.insert("range_start".into(), 2.into());
                        object.insert("range_end".into(), 8.into());
                    }
                    _ => {}
                }
            }
        }
        parts
    }
    // Store component fixture: admit real Run/Turn/Part owners and bind
    // independent original AgentItems through an applied source stamp. This
    // does not claim a real provider/Core execution (covered by integration).
    fn seed(path: &std::path::Path, parts: Vec<Value>, originals: Vec<OriginalTool>) -> NolaStore {
        let mut store = NolaStore::open(path).unwrap();
        let mut bindings=vec![AssistantTurnBinding::Admission {turn_id:"r".into(),assistant_message_id:"m".into()},
            AssistantTurnBinding::Part {turn_id:"r".into(),assistant_message_id:"m".into(),part_id:"admission-text".into()}];
        let mutations=parts.into_iter().enumerate().map(|(index,raw)| {
            let id=raw.get("id").or_else(||raw.get("part_id")).unwrap().as_str().unwrap().to_owned();
            bindings.push(AssistantTurnBinding::Part {turn_id:"r".into(),assistant_message_id:"m".into(),part_id:id.clone()});
            PartMutation::Upsert {part_id:id,kind:raw["kind"].as_str().unwrap().into(),ordinal:index as i64+2,
                state:"completed".into(),content_text:String::new(),content_json:Some(raw.to_string()),
                content_digest:None,created_at_ms:1,updated_at_ms:2}
        }).collect();
        store.begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision:None,expected_parent_message_id:None,initial_events:vec![],
            conversation_id:"c".into(),conversation_title:"History".into(),user_message_id:"u".into(),
            user_part_id:"u-text".into(),user_text:"Historical request".into(),user_content_json:None,
            additional_user_parts:vec![],assistant_message_id:"m".into(),assistant_part_id:"admission-text".into(),
            initial_assistant_part_mutations:mutations,assistant_turn_bindings:bindings,run_id:"r".into(),
            context_checkpoint_id:None,checkpoint_revision:1,checkpoint_event_cursor:0,checkpoint_json:"{}".into(),
            invocation_config_json:"{}".into(),invocation_config_digest:"fixture".into(),outbox_effects:vec![],
            model_attempt_mutations:vec![],delegated_invocation_mutations:vec![],delegated_permission_mutations:vec![],
            memory_usage_entries:vec![],created_at_ms:1,
        }).unwrap();
        let mut transition=CommitTransitionInput {
            transition_id:"historical-tool-sources".into(),run_id:"r".into(),expected_checkpoint_revision:1,
            checkpoint_revision:2,checkpoint_event_cursor:0,checkpoint:CheckpointWrite::Baseline {checkpoint_json:"{}".into()},
            submission_reduction_json:None,host_event:None,events:vec![],ensured_messages:vec![],
            part_message_owners:Default::default(),assistant_turn_bindings:vec![],part_mutations:vec![PartMutation::Close {
                part_id:"admission-text".into(),state:"completed".into(),content_json:None,content_digest:None,updated_at_ms:2,
            }],outbox_effects:vec![],
            model_attempt_mutations:vec![],delegated_invocation_mutations:vec![],delegated_permission_mutations:vec![],
            effect_delivery_attempt_mutations:vec![],acknowledged_effect_ids:vec![],memory_usage_entries:vec![],
            run_status:None,assistant_status:Some("completed".into()),assistant_status_message_id:None,committed_at_ms:2,
        };
        super::terminal_sources::add_sources(&store,&mut transition,"c","r","u",false);
        store.begin_write_unit_of_work().unwrap();
        assert!(store.commit_transition(&transition).unwrap().applied);
        let stamp=AssistantSourceCommitStamp::Transition {transition_id:transition.transition_id.clone()};
        let identity=nola_domain::assistant_journal::AssistantJournalIdentity {
            source_store_epoch:store.assistant_source_epoch().unwrap(),conversation_id:"c".into(),
            run_id:"r".into(),assistant_message_id:"m".into(),
        };
        for original in originals {
            let owner=store.assistant_owner_for_part("c","r",&original.part).unwrap().unwrap();
            let call=AgentItem::ToolCall {id:ItemId::from_string(format!("{}:call",original.part)),
                call_id:CallId::from_string(original.call.clone()),name:"file.read".into(),arguments:original.arguments};
            let result=AgentItem::ToolResult {id:ItemId::from_string(format!("{}:result",original.part)),
                call_id:CallId::from_string(original.call.clone()),name:"file.read".into(),output:json!({"ok":true}),
                is_error:false,full_output_ref:None,artifacts:vec![]};
            for (kind,item) in [(ToolSourceKind::ToolCall,call),(ToolSourceKind::ToolResult,result)] {
                if kind==ToolSourceKind::ToolResult && !original.result_present {continue;}
                store.bind_tool_source(&stamp,&identity,&ToolSourceInput {
                    source_item_id:item.id().as_str().into(),kind,call_id:original.call.clone(),name:"file.read".into(),
                    owner:owner.clone(),body_json:serde_json::to_string(&item).unwrap(),
                }).unwrap();
            }
        }
        super::terminal_sources::publish_and_seal(&mut store,&transition,"c");
        store.commit_write_unit_of_work().unwrap();
        assert!(store.local_assistant_run_terminal_seal("c","r").unwrap().is_some(),
            "the history fixture must commit its real journal frontier and typed terminal seal");
        store
    }
    fn context(store: &NolaStore, conversation: &str) -> Vec<AgentItem> {
        let conversation = store.load_conversation(conversation).unwrap().unwrap();
        let (items, owners, _) =
            crate::durable_conversation_items(store, &conversation, None).unwrap();
        assert!(owners.values().all(|owner| conversation
            .messages
            .iter()
            .any(|message| message.id == owner.message_id)));
        items
    }
    fn evidence(items: &[AgentItem], kind: &str) -> Option<Value> {
        items.iter().find_map(|item| match item {
            AgentItem::Message {
                role: AgentRole::Assistant,
                content,
                ..
            } => {
                let value: Value = serde_json::from_str(content).ok()?;
                (value["kind"] == kind).then_some(value)
            }
            _ => None,
        })
    }
    fn assert_references(items: &[AgentItem]) -> (String, String) {
        let source = evidence(items, "historical_source").expect("source data");
        let citation = evidence(items, "historical_citation").expect("citation data");
        let call = items
            .iter()
            .find_map(|item| match item {
                AgentItem::ToolCall { call_id, .. } => Some(call_id.as_str()),
                _ => None,
            })
            .unwrap();
        assert_eq!(citation["origin_tool_call_id"], call);
        assert_eq!(citation["source_id"], source["source_id"]);
        assert_eq!(citation["uri"], "file://original");
        assert_ne!(source["source_id"], "old-source");
        assert_ne!(citation["citation_id"], "old-citation");
        assert!(items.iter().any(
            |item| matches!(item, AgentItem::ToolResult{call_id,..} if call_id.as_str()==call)
        ));
        (
            call.to_owned(),
            source["source_id"].as_str().unwrap().to_owned(),
        )
    }
    #[test]
    fn actual_context_resolves_forward_references_in_both_historical_schemas() {
        for view in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = seed(&dir.path().join("history.db"), parts(view), originals());
            let items = context(&store, "c");
            assert_eq!(items.len(), 5); // Real admitted user plus four historical items.
            assert_references(&items);
        }
    }
    #[test]
    fn unresolved_ambiguous_and_deferred_calls_never_gain_citation_bindings() {
        for mode in [
            "missing_call",
            "duplicate_call",
            "deferred_call",
            "missing_source",
            "duplicate_source",
            "wrong_uri",
            "unverified_citation",
            "unbound_display",
            "missing_original_result",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut parts = parts(false);
            let mut original_items=originals();
            match mode {
                "missing_call" => {
                    parts.pop();
                    original_items.clear();
                }
                "duplicate_call" => {
                    let mut duplicate = parts[2].clone();
                    duplicate["id"] = "duplicate".into();
                    parts.push(duplicate);
                    original_items.push(original_tool("duplicate","old-call",json!({"path":"original"})));
                }
                "deferred_call" => {
                    parts[2]["input"] = json!({"large":"x".repeat(12000)});
                    original_items[0].arguments=json!({"large":"x".repeat(12000)});
                }
                "missing_source" => {
                    parts.remove(1);
                }
                "duplicate_source" => {
                    let mut duplicate = parts[1].clone();
                    duplicate["id"] = "duplicate".into();
                    parts.push(duplicate);
                }
                "wrong_uri" => {
                    parts[0]["uri"] = "file://other".into();
                }
                "unverified_citation" => {
                    parts[0]["turn_id"] = "foreign-turn".into();
                }
                "unbound_display" => original_items.clear(),
                "missing_original_result" => original_items[0].result_present=false,
                _ => unreachable!(),
            }
            let store = seed(&dir.path().join("history.db"), parts, original_items);
            let items = context(&store, "c");
            assert!(evidence(&items, "historical_citation").is_none(), "{mode}");
            assert!(
                items
                    .iter()
                    .any(|item| matches!(item, AgentItem::Message{content,..}
                if content.contains("unsupported reference") || content.contains("omitted"))),
                "{mode}"
            );
            if matches!(mode, "duplicate_call" | "deferred_call" | "missing_call" | "unbound_display" | "missing_original_result") {
                assert!(
                    !items.iter().any(|item| matches!(
                        item,
                        AgentItem::ToolCall { .. } | AgentItem::ToolResult { .. }
                    )),
                    "{mode}"
                );
            }
        }
    }
    #[test]
    fn two_generation_branch_rebinds_references_after_sources_are_deleted_and_store_reopens() {
        for view in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("history.db");
            let mut store = seed(&path, parts(view), originals());
            let original = store.load_conversation("c").unwrap().unwrap();
            let raw: Vec<_> = original.messages.iter().find(|message|message.role=="assistant").unwrap()
                .parts
                .iter()
                .map(|part| part.content_json.clone())
                .collect();
            let initial = assert_references(&context(&store, "c"));
            let mut previous = initial;
            for (source, target) in [("c", "first"), ("first", "second")] {
                let source = store.load_conversation(source).unwrap().unwrap();
                store
                    .create_conversation_branch(&CanonicalBranchIntent {
                        operation_id: format!("branch-{target}"),
                        source_conversation_id: source.id.clone(),
                        expected_source_revision: source.revision,
                        through_message_id: source.messages.iter().find(|message|message.role=="assistant").unwrap().id.clone(),
                        conversation: CanonicalBranchTarget {
                            id: target.into(),
                            title: "Branch".into(),
                            settings_json: None,
                            created_at_ms: 3,
                            updated_at_ms: 3,
                        },
                        committed_at_ms: 3,
                    })
                    .unwrap();
                let mapped = assert_references(&context(&store, target));
                assert_ne!(mapped.0, previous.0);
                assert_ne!(mapped.1, previous.1);
                previous = mapped;
                assert!(store.delete_conversation(&source.id).unwrap());
            }
            drop(store);
            let store = NolaStore::open(&path).unwrap();
            assert_eq!(assert_references(&context(&store, "second")), previous);
            let target = store.load_conversation("second").unwrap().unwrap();
            assert_eq!(
                target.messages.iter().find(|message|message.role=="assistant").unwrap()
                    .parts
                    .iter()
                    .map(|part| part.content_json.clone())
                    .collect::<Vec<_>>(),
                raw
            );
        }
    }
    #[test]
    fn historical_artifacts_and_tasks_are_data_without_executable_items() {
        for raw in [
            json!({"kind":"artifact","id":"p","message_id":"m","turn_id":"r",
                "artifact":{"uri":"file://original","name":"Report","mime_type":"text/plain"}}),
            json!({"kind":"step","id":"p","message_id":"m","turn_id":"r",
                "title":"Previous work","status":"completed"}),
            json!({"kind":"attachment","part_id":"p","revision":1,
                "attachment_id":"old-attachment","name":"Report","uri":"file://original"}),
            json!({"kind":"task","part_id":"p","revision":1,
                "task_id":"old-task","display_key":"Previous work","state":"completed","cancel_requested":false}),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let kind = raw["kind"].as_str().unwrap().to_owned();
            let store = seed(&dir.path().join("history.db"), vec![raw], vec![]);
            let items = context(&store, "c");
            let data =
                evidence(&items, &format!("historical_{kind}")).expect("typed historical data");
            assert_eq!(data["part_id"], "p");
            assert!(data.get("task_id").is_none());
            assert!(data.get("attachment_id").is_none());
            assert_eq!(items.len(), 2); // The admitted user is not historical evidence.
            assert!(matches!(
                items[1],
                AgentItem::Message {
                    role: AgentRole::Assistant,
                    ..
                }
            ));
        }
    }
    #[test]
    fn semantic_budget_exhaustion_keeps_explicit_context_notes_and_complete_tool_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let parts = (0..64)
            .map(|index| {
                json!({"kind":"tool","part_id":format!("p{index}"),
            "execution_source":{"kind":"model"},
            "revision":1,"tool_call_id":format!("call{index}"),"capability_id":"file.read",
            "state":"completed","title":"Read","origin":{"kind":"builtin"},"artifacts":[],
            "input":{"path":"x".repeat(1024)},"output":{"ok":true},"affected_resources":[]})
            })
            .collect();
        let originals=(0..64).map(|index|original_tool(&format!("p{index}"),&format!("call{index}"),
            json!({"path":"x".repeat(1024)}))).collect();
        let store = seed(&dir.path().join("history.db"), parts, originals);
        let conversation = store.load_conversation("c").unwrap().unwrap();
        let (mut items, mut owners, _) =
            crate::durable_conversation_items(&store, &conversation, None).unwrap();
        assert!(items.iter().any(
            |item| matches!(item,AgentItem::Message{content,..} if content.contains("NotLoaded"))
        ));
        crate::historical_part_projection::bound_context("c", &mut items, &mut owners, 32768)
            .unwrap();
        assert!(serde_json::to_vec(&(&items, &owners)).unwrap().len() <= 32768);
        let calls: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                AgentItem::ToolCall { call_id, .. } => Some(call_id),
                _ => None,
            })
            .collect();
        let results: Vec<_> = items
            .iter()
            .filter_map(|item| match item {
                AgentItem::ToolResult { call_id, .. } => Some(call_id),
                _ => None,
            })
            .collect();
        assert_eq!(calls, results);
    }
    #[test]
    fn canonical_part_ids_cannot_alias_generated_tool_items_during_context_pruning() {
        let dir = tempfile::tempdir().unwrap();
        let mut original = parts(false);
        original.push(
            json!({"kind":"text","id":"large","message_id":"m","turn_id":"r",
            "text":"x".repeat(6500),"is_complete":true}),
        );
        let mut store = seed(&dir.path().join("history.db"), original, originals());
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "c".into(),
                expected_message_count: 2,
                updated_at_ms: 3,
                messages: vec![MessageInput {
                    id: "new".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 3,
                    created_at_ms: 3,
                    completed_at_ms: Some(3),
                    parts: vec![MessagePartInput {
                        // This used to collide with the old tool's generated call item.
                        id: "tool:call".into(),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: "Latest message".into(),
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 3,
                        updated_at_ms: 3,
                    }],
                }],
            })
            .unwrap();
        let conversation = store.load_conversation("c").unwrap().unwrap();
        let (mut items, mut owners, _) =
            crate::durable_conversation_items(&store, &conversation, None).unwrap();
        assert_eq!(items.len(), owners.len());
        let all_ids: std::collections::BTreeSet<_> =
            items.iter().map(|item| item.id().as_str()).collect();
        assert_eq!(all_ids.len(), items.len());
        assert!(items
            .iter()
            .any(|item| matches!(item, AgentItem::ToolCall { .. })));
        crate::historical_part_projection::bound_context("c", &mut items, &mut owners, 4096)
            .unwrap();
        assert!(items.iter().any(
            |item| matches!(item,AgentItem::Message{content,..} if content=="Latest message")
        ));
        assert!(!items.iter().any(|item| matches!(
            item,
            AgentItem::ToolCall { .. } | AgentItem::ToolResult { .. }
        )));
        assert!(owners.values().all(|owner| owner.message_id == "new"));
        assert!(serde_json::to_vec(&(&items, &owners)).unwrap().len() <= 4096);
    }
}
