use nola_store::{BeginTurnInput, NolaStore, OutboxEffectInput, StoredConversation};
use tempfile::tempdir;

#[path = "support/conversation_deletion_fixture.rs"]
mod conversation_deletion_fixture;

fn begin_input() -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
        conversation_id: "conversation-sync-1".into(),
        conversation_title: "Durable conversation".into(),
        user_message_id: "message-user-1".into(),
        user_part_id: "part-user-1".into(),
        user_text: "Preserve this complete request".into(),
        user_content_json: None,
        additional_user_parts: Vec::new(),
        assistant_message_id: "message-assistant-1".into(),
        assistant_part_id: "part-assistant-1".into(),
        assistant_turn_bindings: vec![],
        run_id: "run-1".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        // The later cancellation is a submission in this admitted Core session.
        checkpoint_json: serde_json::json!({
            "session_id": "fixture-core:run-1", "revision": 1,
            "state": {"session_id": "fixture-core:run-1"}
        }).to_string(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "conversation-config-digest".into(),
        outbox_effects: vec![OutboxEffectInput {
            delegated_invocation_id: None,
            model_attempt_id: None,
            effect_id: "effect-model-1".into(),
            request_id: "request-model-1".into(),
            effect_kind: "model_request".into(),
            write: outbox_fixture::model("effect-model-1", "request-model-1", "text", "reasoning"),
        }],
        created_at_ms: 1_000,
    }
}

#[test]
fn conversation_projections_and_tombstone_survive_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");

    {
        let mut store = NolaStore::open(&database).unwrap();
        store.begin_turn(&begin_input()).unwrap();
        let changes = store.pending_sync_changes(20).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].entity_type, "conversation");
        assert_eq!(changes[0].entity_id, "conversation-sync-1");
        assert_eq!(changes[0].revision, 1);
        let projection: StoredConversation =
            serde_json::from_str(&changes[0].payload_json).unwrap();
        assert_eq!(projection.messages.len(), 2);
        assert_eq!(
            projection.messages[0].parts[0].content_text,
            "Preserve this complete request"
        );

        assert!(store
            .rename_conversation("conversation-sync-1", "Renamed", None, 2_000)
            .unwrap());
    }

    {
        let mut reopened = NolaStore::open(&database).unwrap();
        let changes = reopened.pending_sync_changes(20).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].entity_type, "conversation");
        assert_eq!(changes[0].revision, 1);
        let original: StoredConversation = serde_json::from_str(&changes[0].payload_json).unwrap();
        assert_eq!(
            original.messages[0].parts[0].content_text,
            "Preserve this complete request"
        );
        assert_eq!(changes[1].entity_type, "conversation_metadata");
        assert_eq!(changes[1].revision, 2);
        let metadata: nola_domain::ConversationMetadataReplica =
            serde_json::from_str(&changes[1].payload_json).unwrap();
        assert_eq!(metadata.title(), "Renamed");
        assert_eq!(metadata.base_content_revision(), 1);
        assert!(changes[1].payload_json.len() < 1024);
        assert_eq!(
            reopened
                .load_conversation("conversation-sync-1")
                .unwrap()
                .unwrap()
                .title,
            "Renamed"
        );

        conversation_deletion_fixture::cancel_before_delete(&mut reopened, &begin_input());
        assert!(reopened.delete_conversation("conversation-sync-1").unwrap());
    }

    {
        let reopened = NolaStore::open(&database).unwrap();
        assert!(reopened
            .load_conversation("conversation-sync-1")
            .unwrap()
            .is_none());
        let changes = reopened.pending_sync_changes(20).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].revision, 4);
        assert!(changes[0].deleted);
        assert_eq!(changes[0].payload_json, "null");
    }
}
#[path = "support/outbox.rs"]
mod outbox_fixture;
