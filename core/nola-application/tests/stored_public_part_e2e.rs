// SPDX-License-Identifier: Apache-2.0
//! Canonical transcript admission/query/cold reopen retains bounded public facts.
use nola_application::{
    load_conversation_projection_page_v1, load_conversation_projection_v1,
    load_conversation_projection_window_v1,
};
use nola_domain::{HistoricalProjectionStatus, MessagePartView, MessageView, SemanticValue};
use nola_store::NolaStore;
use serde_json::{json, Value};

const CONVERSATION: &str = "stored-public-conversation";
const MESSAGE: &str = "stored-public-assistant";

fn open(store: &mut NolaStore) {
    store
        .create_conversation(
            &nola_store_port::conversation_profile::CreateConversationInput {
                id: CONVERSATION.into(),
                title: "Public facts".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        )
        .unwrap();
}

fn tool(index: usize, output: Option<Value>) -> Value {
    let mut value = json!({
        "kind":"tool", "part_id":format!("tool-{index}"), "revision":2,
        "execution_source":{"kind":"model"},
        "tool_call_id":format!("call-{index}"), "capability_id":"files.write",
        "title":"真实工具标题 α", "summary":"", "state":"completed",
        "input":{"path":"actual input"},
        "origin":{"kind":"mcp", "provider_title":"提供方"},
        "permission":{"request_id":"approval", "permission_domain":"files", "rationale":"仅展示", "decision":"allow_once"},
        "output_preview":"独立预览", "full_output":{"message_id":MESSAGE, "part_id":format!("tool-{index}"), "tool_call_id":format!("call-{index}")},
        "artifacts":[{"uri":"file:///output.pdf", "name":"PDF", "mime_type":"application/pdf", "operation":"create"}],
        "started_at_ms":0, "finished_at_ms":u64::MAX, "affected_resources":[]
    });
    if let Some(output) = output {
        value["output"] = output;
    }
    value
}

fn commit(store: &mut NolaStore, parts: Vec<Value>) -> MessageView {
    // Canonical transcript fixtures exercise persisted public semantics without
    // reviving the removed full-aggregate remote write protocol.
    let message: MessageView = serde_json::from_value(json!({
        "message_id":MESSAGE,"role":"assistant","state":"completed","revision":2,"created_at_ms":1,"parts":parts
    })).unwrap();
    for part in &message.parts {
        part.validate_public_semantics(&nola_domain::MessageId(MESSAGE.into()))
            .unwrap();
    }
    let canonical_parts = parts
        .into_iter()
        .enumerate()
        .map(|(index, part)| nola_domain::MessagePartInput {
            id: part["part_id"].as_str().unwrap().into(),
            kind: part["kind"].as_str().unwrap().into(),
            ordinal: index as i64 + 1,
            state: part["state"].as_str().unwrap_or("completed").into(),
            content_text: part["markdown"]
                .as_str()
                .or_else(|| part["summary"].as_str())
                .unwrap_or("")
                .into(),
            content_json: Some(part.to_string()),
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 3,
        })
        .collect();
    use nola_store_port::transcript_write::{AppendMessagesInput, MessageInput};
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: CONVERSATION.into(),
            expected_message_count: 0,
            updated_at_ms: 3,
            messages: vec![
                MessageInput {
                    id: "user".into(),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(1),
                    parts: vec![],
                },
                MessageInput {
                    id: MESSAGE.into(),
                    run_id: None,
                    role: "assistant".into(),
                    status: "completed".into(),
                    ordinal: 2,
                    created_at_ms: 1,
                    completed_at_ms: Some(3),
                    parts: canonical_parts,
                },
            ],
        })
        .unwrap();
    message
}

#[test]
fn canonical_commit_live_query_and_cold_reopen_preserve_typed_public_parts() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("public.db");
    let mut store = NolaStore::open(&database).unwrap();
    open(&mut store);
    let mut parts: Vec<_> = [
        None,
        Some(Value::Null),
        Some(json!("")),
        Some(json!([])),
        Some(json!({})),
        Some(json!({"number":u64::MAX})),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, output)| tool(index, output))
    .collect();
    parts.extend([
        json!({"kind":"reasoning", "part_id":"reasoning", "revision":2, "state":"completed", "title":"独立推理标题", "summary":"独立正文"}),
        json!({"kind":"source", "part_id":"source-part", "revision":2, "source_id":"source", "title":"Evidence", "uri":"",
            "evidence":{"kind":"memory", "usage":{"record_id":"record", "component_id":"component", "namespace":{"kind":"workspace"}, "usage_count":u64::MAX}}}),
        json!({"kind":"citation", "part_id":"citation", "revision":2, "citation_id":"real-citation", "source_id":"source", "title":"Citation", "uri":"",
            "mime_type":"application/pdf", "origin_tool_call_id":"call-0", "location":{"page":0, "section":"方法"}, "page":0, "section":"方法", "range_start":7, "range_end":12}),
        json!({"kind":"citation", "part_id":"empty-location", "revision":2, "citation_id":"empty-location", "source_id":"source", "title":"", "uri":" ", "origin_tool_call_id":"call-0", "location":{}}),
        json!({"kind":"attachment", "part_id":"attachment", "revision":2, "attachment_id":"artifact", "name":"PDF", "uri":"file:///output.pdf", "operation":"create"}),
        json!({"kind":"text", "part_id":"answer", "revision":2, "markdown":"Visible answer", "is_final":true, "review":"approved"}),
        json!({"kind":"text", "part_id":"withheld", "revision":2, "markdown":"", "is_final":true, "review":"pending"}),
    ]);
    let expected = commit(&mut store, parts).parts;
    let mut canonical = None;
    for application in [store, NolaStore::open(&database).unwrap()] {
        let full = load_conversation_projection_v1(&application, CONVERSATION).unwrap();
        let window =
            load_conversation_projection_window_v1(&application, CONVERSATION, None, 1, 32768)
                .unwrap();
        let page = load_conversation_projection_page_v1(&application, CONVERSATION, 1, 1).unwrap();
        for projection in [full, window.projection, page] {
            let actual = &projection
                .messages
                .iter()
                .find(|message| message.message_id.0 == MESSAGE)
                .unwrap()
                .parts;
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(&expected) {
                let mut expected = serde_json::to_value(expected).unwrap();
                expected["revision"] = json!(projection.revision);
                assert_eq!(serde_json::to_value(actual).unwrap(), expected);
                assert!(!matches!(actual, MessagePartView::Historical { .. }));
                actual
                    .validate_public_semantics(&nola_domain::MessageId(MESSAGE.into()))
                    .unwrap();
            }
            if let Some(previous) = &canonical {
                assert_eq!(actual, previous);
            }
            canonical = Some(actual.clone());
        }
    }
}

#[test]
fn oversized_remote_result_remains_deferred_original_not_empty_typed_output() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("deferred.db");
    let mut store = NolaStore::open(&database).unwrap();
    open(&mut store);
    let output = "actual result🙂".repeat(20_000);
    commit(&mut store, vec![tool(0, Some(json!({"result":output})))]);
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    let window =
        load_conversation_projection_window_v1(&store, CONVERSATION, None, 1, 1024).unwrap();
    let MessagePartView::Historical { projection, .. } = &window.projection.messages[0].parts[0]
    else {
        panic!("Deferred original must not become a complete/null/empty tool result")
    };
    assert_eq!(projection.owner.message_id(), MESSAGE);
    assert_eq!(projection.owner.part_id(), "tool-0");
    assert_eq!(projection.status, HistoricalProjectionStatus::Deferred);
    assert!(matches!(
        projection.fields["output"],
        SemanticValue::Deferred { .. }
    ));
    let json = store
        .load_conversation(CONVERSATION)
        .unwrap()
        .unwrap()
        .messages
        .into_iter()
        .find(|message| message.id == MESSAGE)
        .unwrap()
        .parts
        .remove(0)
        .content_json
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&json).unwrap()["output"]["result"],
        output
    );
}

#[test]
fn unknown_private_fields_in_stored_public_source_cannot_be_filtered_into_valid_cards() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("unknown.db");
    let mut store = NolaStore::open(&database).unwrap();
    open(&mut store);
    commit(&mut store, vec![tool(0, Some(Value::Null))]);
    drop(store);
    // Isolated corruption fixture exercises the actual persisted-source reader,
    // rather than an already-filtered public dictionary.
    let connection = rusqlite::Connection::open(&database).unwrap();
    let mut source = tool(0, Some(Value::Null));
    source["private_grant"] = json!({"token":"PRIVATE_GRANT_MARKER"});
    connection
        .execute(
            "UPDATE message_parts SET content_json=?1 WHERE id='tool-0'",
            rusqlite::params![source.to_string()],
        )
        .unwrap();
    drop(connection);
    let store = NolaStore::open(&database).unwrap();
    let projection = load_conversation_projection_window_v1(&store, CONVERSATION, None, 1, 32768)
        .unwrap()
        .projection;
    let MessagePartView::Historical {
        projection: original,
        ..
    } = &projection.messages[0].parts[0]
    else {
        panic!("Unknown source fields cannot silently become a strict public Tool")
    };
    assert_eq!(original.owner.part_id(), "tool-0");
    assert!(
        matches!(&original.status, HistoricalProjectionStatus::Unsupported { code } if code == "schema_unknownfield")
    );
    assert!(original.fields.is_empty());
    assert!(!serde_json::to_string(&projection)
        .unwrap()
        .contains("PRIVATE_GRANT_MARKER"));
}
