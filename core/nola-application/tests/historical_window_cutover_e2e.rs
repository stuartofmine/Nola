// SPDX-License-Identifier: Apache-2.0
//! Production window loaders use canonical typed history, never JSON previews.
use nola_application::load_conversation_projection_window_v1;
use nola_domain::{
    CanonicalBranchIntent, CanonicalBranchTarget, HistoricalProjectionStatus, MessagePartView,
    SemanticIdentityAssessment, SemanticValue,
};
use nola_store::NolaStore;
use serde_json::json;

fn seed(path: &std::path::Path, body: &str, kind: &str) {
    drop(NolaStore::open(path).unwrap());
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Historical',1,1,2); INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,2)").unwrap();
    db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms) VALUES('p','m',?1,1,'completed','plain preview',?2,1,2)", rusqlite::params![kind, body]).unwrap();
}
#[test]
fn window_reads_ten_million_byte_original_json_as_deferred_historical_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.db");
    let body = json!({"kind":"tool","execution_source":{"kind":"model"},"part_id":"p","revision":1,"tool_call_id":"call","capability_id":"read","title":"Read historical output","origin":{"kind":"builtin"},"state":"completed","input":{},"output":"x".repeat(10_000_000),"affected_resources":[],"artifacts":[]}).to_string();
    seed(&path, &body, "tool");
    let store = NolaStore::open(&path).unwrap();
    let raw = store
        .load_conversation_message_window("c", None, 32, 1024)
        .unwrap()
        .unwrap();
    assert!(raw.conversation.messages[0].parts[0].content_json.is_none());
    let window = load_conversation_projection_window_v1(&store, "c", None, 32, 1024).unwrap();
    let MessagePartView::Historical {
        part_id,
        projection,
        ..
    } = &window.projection.messages[0].parts[0]
    else {
        panic!("original JSON must not degrade to preview text")
    };
    assert_eq!(part_id.0, "p");
    assert_eq!(
        projection.binding,
        Some(SemanticIdentityAssessment::Canonical)
    );
    assert_eq!(projection.status, HistoricalProjectionStatus::Deferred);
    assert!(matches!(
        projection.fields.get("output"),
        Some(SemanticValue::Deferred { .. })
    ));
    assert!(serde_json::to_vec(&window).unwrap().len() < 32768);
}
#[test]
fn branch_window_uses_canonical_target_and_unsupported_history_remains_a_node() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("branch.db");
    let body = json!({"kind":"reasoning","part_id":"p","revision":1,"title":"Historical reasoning","state":"completed","summary":"Historical evidence"}).to_string();
    seed(&path, &body, "reasoning");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation_branch(&CanonicalBranchIntent {
            operation_id: "branch".into(),
            source_conversation_id: "c".into(),
            expected_source_revision: 1,
            through_message_id: "m".into(),
            conversation: CanonicalBranchTarget {
                id: "derived".into(),
                title: "Derived".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        })
        .unwrap();
    let window = load_conversation_projection_window_v1(&store, "derived", None, 32, 1024).unwrap();
    let MessagePartView::Historical {
        part_id,
        projection,
        ..
    } = &window.projection.messages[0].parts[0]
    else {
        panic!("branch must use typed historical node")
    };
    assert_ne!(part_id.0, "p");
    assert_eq!(part_id.0, projection.owner.part_id());
    assert_eq!(projection.owner.conversation_id(), "derived");
    assert_eq!(
        projection.binding,
        Some(SemanticIdentityAssessment::VerifiedLocalDerivation)
    );
    assert_eq!(projection.status, HistoricalProjectionStatus::Supported);
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE message_parts SET content_json='{\"kind\":\"future_kind\",\"part_id\":\"p\"}' WHERE id='p'", []).unwrap();
    let store = NolaStore::open(&path).unwrap();
    let unsupported = load_conversation_projection_window_v1(&store, "c", None, 32, 1024).unwrap();
    let MessagePartView::Historical { projection, .. } =
        &unsupported.projection.messages[0].parts[0]
    else {
        panic!("unsupported original must remain visible")
    };
    assert!(matches!(
        projection.status,
        HistoricalProjectionStatus::Unsupported { .. }
    ));
    assert!(projection.fields.is_empty());
}
