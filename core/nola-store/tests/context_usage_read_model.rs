// SPDX-License-Identifier: Apache-2.0
// Read-model fixtures exercise extraction and source precedence, not Host execution.

use nola_store::{CreateConversationInput, NolaStore, SaveCheckpointInput};
use serde_json::{json, Value};

#[test]
fn local_presentation_inputs_do_not_return_model_working_set_or_memory() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "local".into(),
            title: "Local".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let checkpoint = json!({
        "session_id": "local", "last_event_cursor": {"sequence": 2},
        "state": {
            "session_id": "local",
            "conversation": {
                "projection": {"revision": 1, "order": [], "parts": {}},
                "model_working_set": "x".repeat(10_000_000),
                "context_window": {
                    "config": {}, "used_tokens": 123, "usage_origin": "estimated"
                }
            },
            "memory_projection": {"private": "not part of the timeline"},
            "task_projection": {"revision": 0, "order": [], "tasks": {}},
            "skill_runtime": {"revision": 0, "catalog": {"revision": 0}, "runs": {}}
        }
    });
    store
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "local".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: 2,
            checkpoint_json: checkpoint.to_string(),
            updated_at_ms: 2,
        })
        .unwrap();
    let inputs = store
        .load_local_presentation_inputs("local", "[]", None)
        .unwrap()
        .unwrap();
    assert!(inputs.len() < 2048);
    assert!(!inputs.contains("model_working_set"));
    assert!(!inputs.contains("not part of the timeline"));
    let inputs: Value = serde_json::from_str(&inputs).unwrap();
    assert_eq!(inputs["cursor"], 2);
    assert!(inputs.get("used_tokens").is_none(),
        "checkpoint-only estimates are not authoritative committed Run usage");
    assert_eq!(inputs["projection"]["revision"], 1);
}
