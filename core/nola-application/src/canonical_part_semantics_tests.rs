// SPDX-License-Identifier: Apache-2.0
//! Real canonical Upsert originals, independent of Core checkpoint survival.
use super::*;
use crate::historical_part_projection;
use nola_domain::{CanonicalPartOwner, HistoricalProjectionStatus, SemanticValue, StructuredValue};
use nola_agent_engine::agent::parts::{PartOwnership, ToolPart};
use nola_store::{
    AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput, NolaStore,
};

fn tool(output: Option<serde_json::Value>, retained: bool) -> AgentPart {
    AgentPart::Tool(ToolPart {
        execution_source: Default::default(),
        id: "p".into(),
        ownership: PartOwnership {
            message_id: "m".into(),
            turn_id: "r".into(),
        },
        call_id: "call".into(),
        capability_id: "file.read".into(),
        title: "title".into(),
        summary: Some(String::new()),
        origin: Default::default(),
        state: ToolPartState::Completed,
        input: serde_json::json!({"path":"file"}),
        raw_input: "PRIVATE_RAW_MARKER".into(),
        permission: None,
        output_preview: Some("independent preview".into()),
        structured_output: output,
        full_output_ref: retained.then(|| "/PRIVATE_OUTPUT_PATH_MARKER".into()),
        artifacts: vec![],
        started_at_ms: Some(0),
        finished_at_ms: Some(u64::MAX),
    })
}

fn persist(database: &std::path::Path, part: &AgentPart) {
    let mut store = NolaStore::open(database).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Originals".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let PartMutation::Upsert {
        part_id,
        kind,
        ordinal,
        state,
        content_text,
        content_json,
        content_digest,
        created_at_ms,
        updated_at_ms,
    } = part_upsert(part, 1, 2).unwrap()
    else {
        unreachable!()
    };
    assert_eq!(
        content_json.as_deref(),
        Some(serde_json::to_string(part).unwrap().as_str()),
        "canonical original must not be a truncated diagnostic preview"
    );
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
                    id: part_id,
                    kind,
                    ordinal,
                    state,
                    content_text,
                    content_json,
                    content_digest,
                    created_at_ms,
                    updated_at_ms,
                }],
            }],
        })
        .unwrap();
}

#[test]
fn canonical_cold_history_retains_code_cell_provenance_without_a_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("code-history.db");
    let source = nola_domain::code_mode::CodeItemSource::CodeCell {
        cell: nola_domain::code_mode::CodeCellIdentity {
            host_id:"host".into(), conversation_id:"c".into(), turn_id:"original-core-turn".into(),
            parent_call_id:"parent".into(), cell_id:"cell".into(), generation:"g".into(),
        },
    };
    let mut part = tool(Some(serde_json::json!({"intermediate":"retained"})), false);
    if let AgentPart::Tool(value) = &mut part { value.execution_source = source.clone(); }
    persist(&database, &part);
    drop(part);
    let store = NolaStore::open(&database).unwrap();
    let stored = store.load_conversation("c").unwrap().unwrap();
    let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
    let projection = store.read_historical_part(&owner, stored.revision, 8192).unwrap().unwrap();
    assert_eq!(projection.status, HistoricalProjectionStatus::Supported);
    let value = historical_part_projection::value(&projection, "execution_source").unwrap();
    let recovered: nola_domain::code_mode::CodeItemSource =
        serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap();
    assert_eq!(recovered, source);
    assert!(!recovered.is_model_visible());
}

#[test]
fn canonical_tool_upsert_retains_the_only_large_structured_original_without_a_private_file() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("original.db");
    let expected = serde_json::json!({"body":"唯一结果🙂".repeat(5000), "count":u64::MAX});
    let mut part = tool(Some(expected.clone()), false);
    let AgentPart::Tool(value) = &mut part else {
        unreachable!()
    };
    value.title = "长标题🙂".repeat(1200);
    value.output_preview = Some("独立预览🙂".repeat(1000));
    persist(&database, &part);
    drop(part);
    let store = NolaStore::open(&database).unwrap();
    let stored = store.load_conversation("c").unwrap().unwrap();
    let raw = stored.messages[0].parts[0]
        .content_json
        .as_ref()
        .expect("unbounded export reads the immutable original");
    let original: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(original["structured_output"], expected);
    assert_eq!(original["full_output_ref"], serde_json::Value::Null);
    assert!(original["structured_output"].get("truncated").is_none());
    let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
    let projection = store
        .read_historical_part(&owner, stored.revision, 8192)
        .unwrap()
        .unwrap();
    assert_eq!(projection.status, HistoricalProjectionStatus::Deferred);
    let SemanticValue::Deferred { locator } = &projection.fields["structured_output"] else {
        panic!("large result must retain its original token locator")
    };
    assert!(locator.end_bytes() - locator.start_bytes() > 2048);
    assert!(!projection.fields.contains_key("raw_input"));
    assert!(!projection.fields.contains_key("full_output_ref"));
    assert!(!projection.fields.contains_key("full_output"));
    let wire = serde_json::to_string(&projection).unwrap();
    assert!(wire.len() <= 8192);
    assert!(!wire.contains("PRIVATE_RAW_MARKER"));
}

#[test]
fn canonical_cold_history_preserves_result_absence_null_empty_and_retained_owner() {
    for (index, output) in [
        None,
        Some(serde_json::Value::Null),
        Some(serde_json::json!("")),
        Some(serde_json::json!([])),
        Some(serde_json::json!({})),
    ]
    .into_iter()
    .enumerate()
    {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("result.db");
        persist(&database, &tool(output.clone(), true));
        let store = NolaStore::open(&database).unwrap();
        let revision = store.conversation_revision("c").unwrap().unwrap();
        let owner = CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
        let projection = store
            .read_historical_part(&owner, revision, 8192)
            .unwrap()
            .unwrap();
        assert_eq!(projection.status, HistoricalProjectionStatus::Supported);
        assert_eq!(
            projection
                .fields
                .get("structured_output")
                .map(|field| match field {
                    SemanticValue::Complete { value } => serde_json::to_value(value).unwrap(),
                    _ => panic!("small result deferred"),
                }),
            output,
            "case {index}"
        );
        assert_eq!(
            projection.fields["summary"],
            SemanticValue::Complete {
                value: StructuredValue::String(String::new())
            }
        );
        assert_eq!(
            projection.fields["output_preview"],
            SemanticValue::Complete {
                value: StructuredValue::String("independent preview".into())
            }
        );
        let SemanticValue::Complete { value } = &projection.fields["full_output"] else {
            panic!("retained output owner missing")
        };
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            serde_json::json!({"message_id":"m","part_id":"p","tool_call_id":"call"})
        );
        let wire = serde_json::to_string(&projection).unwrap();
        assert!(!wire.contains("PRIVATE_RAW_MARKER"));
        assert!(!wire.contains("PRIVATE_OUTPUT_PATH_MARKER"));
    }
}
