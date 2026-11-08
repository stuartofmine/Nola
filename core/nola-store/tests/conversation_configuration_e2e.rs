use nola_store::NolaStore;
use nola_store_port::conversation_profile::{
    CreateConversationInput, PatchConversationConfigurationInput,
};
use serde_json::{json, Value};

fn committed(result: nola_domain::conversation_configuration::ConfigurationCommitResult) -> Value {
    serde_json::to_value(result.into_committed().expect("configuration committed")).unwrap()
}
#[test]
fn shared_knowledge_is_sparse_revision_checked_and_preserves_native_markdown() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("knowledge.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    let initial = json!({"isPinned":true, "permissionModeRawValue":"cautious", "nativeField":"keep",
        "modelOverrides":{"configured:m":{"temperature":0.5}},
        "knowledge":{"keyPointsMarkdown":"old facts", "wikiMarkdown":"# Keep\n", "updatedAt":0},
        "conversation":{"knowledgePoints":"preserve owned data"}});
    store.create_conversation(&CreateConversationInput {
        id:"knowledge".into(), title:"Knowledge".into(), revision:1,
        settings_json:Some(initial.to_string()), created_at_ms:1, updated_at_ms:1,
    }).unwrap();
    let input = PatchConversationConfigurationInput {
        conversation_id:"knowledge".into(), expected_revision:1,
        patch_json:json!({"knowledge":{"keyPointsMarkdown":"  事实\n\n"}}).to_string().parse().unwrap(),
        updated_at_ms:978_307_201_500,
    };
    let receipt: Value = committed(store.patch_conversation_configuration(&input).unwrap());
    assert_eq!(receipt["configurationRevision"], 2);
    assert_eq!(receipt["knowledge"], json!({"keyPointsMarkdown":"  事实\n\n", "wikiMarkdown":"# Keep\n", "updatedAt":1.5}));
    assert!(store.patch_conversation_configuration(&input).unwrap().is_conflict());
    let settings: Value = serde_json::from_str(&store.load_conversation_settings("knowledge").unwrap().unwrap()).unwrap();
    for key in ["isPinned", "permissionModeRawValue", "nativeField", "modelOverrides", "conversation"] {
        assert_eq!(settings[key], initial[key]);
    }
    let clear = PatchConversationConfigurationInput {
        expected_revision:2, patch_json:json!({"knowledge":{"wikiMarkdown":""}}).to_string().parse().unwrap(),
        updated_at_ms:978_307_202_000, ..input
    };
    assert!(store.patch_conversation_configuration(&clear).unwrap().is_committed());
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    let settings: Value = serde_json::from_str(&store.load_conversation_settings("knowledge").unwrap().unwrap()).unwrap();
    assert_eq!(settings["knowledge"]["keyPointsMarkdown"], "  事实\n\n");
    assert_eq!(settings["knowledge"]["wikiMarkdown"], "");
    assert_eq!(settings["knowledge"]["updatedAt"], 2.0);
}

#[test]
fn invalid_knowledge_patch_rolls_back_and_absent_document_initializes_both_native_fields() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(&root.path().join("knowledge-validation.sqlite")).unwrap();
    store.create_conversation(&CreateConversationInput {
        id:"knowledge".into(), title:"Knowledge".into(), revision:1,
        settings_json:Some("{}".into()), created_at_ms:1, updated_at_ms:1,
    }).unwrap();
    for knowledge in [json!({}), Value::Null, json!({"wikiMarkdown":null}), json!({"wikiMarkdown":42}),
        json!({"knowledgePoints":"alias"}), json!({"wikiMarkdown":"text", "updatedAt":1})] {
        assert!(json!({"knowledge":knowledge}).to_string()
            .parse::<nola_domain::configuration_patch::ConversationConfigurationPatch>().is_err());
        assert_eq!(store.load_conversation_settings("knowledge").unwrap().unwrap(), "{}");
    }
    let input = PatchConversationConfigurationInput { conversation_id:"knowledge".into(), expected_revision:1,
        patch_json:json!({"knowledge":{"wikiMarkdown":"new"}}).to_string().parse().unwrap(), updated_at_ms:978_307_200_000 };
    let receipt: Value = committed(store.patch_conversation_configuration(&input).unwrap());
    assert_eq!(receipt["configurationRevision"], 2);
    assert_eq!(receipt["knowledge"], json!({"keyPointsMarkdown":"", "wikiMarkdown":"new", "updatedAt":0.0}));
}

#[test]
fn malformed_stored_knowledge_cannot_be_silently_replaced() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(&root.path().join("knowledge-invalid-state.sqlite")).unwrap();
    for (index, knowledge) in [json!("not a document"), json!({"keyPointsMarkdown":"missing wiki"}),
        json!({"keyPointsMarkdown":"facts", "wikiMarkdown":"wiki", "updatedAt":"wrong timestamp"})].into_iter().enumerate() {
        let id = format!("invalid-{index}");
        let settings = json!({"knowledge":knowledge,"isPinned":true}).to_string();
        store.create_conversation(&CreateConversationInput {
            id:id.clone(), title:"Invalid knowledge".into(), revision:1,
            settings_json:Some(settings.clone()), created_at_ms:1, updated_at_ms:1,
        }).unwrap();
        let patch = PatchConversationConfigurationInput { conversation_id:id.clone(), expected_revision:1,
            patch_json:json!({"knowledge":{"wikiMarkdown":"must not replace"}, "isPinned":false}).to_string().parse().unwrap(), updated_at_ms:2 };
        assert!(store.patch_conversation_configuration(&patch).is_err());
        assert_eq!(store.load_conversation_settings(&id).unwrap().unwrap(), settings);
        // The failed transaction did not consume the expected revision.
        let unrelated = PatchConversationConfigurationInput { patch_json:json!({"isPinned":false}).to_string().parse().unwrap(), ..patch };
        assert!(store.patch_conversation_configuration(&unrelated).unwrap().is_committed());
    }
}

#[test]
fn memory_and_retrieval_overrides_are_sparse_revision_checked_and_restore_inheritance() {
    for key in ["memoryEnabledOverride", "retrievalEnabledOverride"] {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(&root.path().join("memory.sqlite")).unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "memory".into(), title: "Memory".into(), revision: 1,
        settings_json: Some(json!({"isPinned":true,"nativeField":"keep"}).to_string()),
        created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    for (index, value) in [json!(true), json!(false), Value::Null].into_iter().enumerate() {
        let revision = index as i64 + 1;
        let patch = PatchConversationConfigurationInput { conversation_id: "memory".into(), expected_revision: revision,
            patch_json: json!({(key):value}).to_string().parse().unwrap(), updated_at_ms: revision + 1 };
        let receipt: Value = committed(store.patch_conversation_configuration(&patch).unwrap());
        assert_eq!(receipt[key], value);
        assert_eq!(receipt["configurationRevision"], revision + 1);
        assert!(store.patch_conversation_configuration(&patch).unwrap().is_conflict());
        let settings: Value = serde_json::from_str(&store.load_conversation_settings("memory").unwrap().unwrap()).unwrap();
        assert_eq!(settings["nativeField"], "keep");
        assert_eq!(settings["isPinned"], true);
        assert_eq!(settings.get(key).is_none(), value.is_null());
    }
    assert!(json!({(key):"true"}).to_string()
        .parse::<nola_domain::configuration_patch::ConversationConfigurationPatch>().is_err());
    }
}

#[test]
fn pinning_is_shared_durable_revision_checked_and_preserves_other_settings() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("pin.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "pin".into(), title: "Pinned conversation".into(), revision: 1,
        settings_json: Some(json!({"permissionModeRawValue":"cautious","modelOverrides":{"configured:m":{"temperature":0.5}},"nativeField":"keep"}).to_string()),
        created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    for (revision, pinned) in [(1, true), (2, false)] {
        let input = PatchConversationConfigurationInput { conversation_id: "pin".into(), expected_revision: revision,
            patch_json: json!({"isPinned":pinned}).to_string().parse().unwrap(), updated_at_ms: revision + 1 };
        let receipt: Value = committed(store.patch_conversation_configuration(&input).unwrap());
        assert_eq!(receipt["isPinned"], pinned);
        assert_eq!(receipt["configurationRevision"], revision + 1);
        assert!(store.patch_conversation_configuration(&input).unwrap().is_conflict());
        drop(store);
        store = NolaStore::open(&database).unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        let raw: String = connection.query_row("SELECT settings_json FROM conversation_settings WHERE conversation_id='pin'", [], |row| row.get(0)).unwrap();
        let settings: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(settings["isPinned"], pinned);
        assert_eq!(settings["nativeField"], "keep");
        assert_eq!(settings["permissionModeRawValue"], "cautious");
        assert_eq!(settings["modelOverrides"]["configured:m"]["temperature"], 0.5);
    }
}

#[test]
fn seed_intent_replaces_atomically_and_null_resets_inheritance() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("seed-intent.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "seed".into(),
            title: "Seed intent".into(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    for (index, seed) in [
        json!({"kind":"value","value":"9223372036854775807"}),
        json!({"kind":"clear"}),
        json!(null),
    ]
    .into_iter()
    .enumerate()
    {
        let revision = index as i64 + 1;
        let patch = PatchConversationConfigurationInput {
            conversation_id: "seed".into(),
            expected_revision: revision,
            patch_json: json!({"modelOverrides":{"configured:model":{"seed":seed}}}).to_string().parse().unwrap(),
            updated_at_ms: revision + 1,
        };
        assert!(store
            .patch_conversation_configuration(&patch)
            .unwrap().is_committed());
        assert!(store
            .patch_conversation_configuration(&patch)
            .unwrap().is_conflict());
        drop(store);
        store = NolaStore::open(&database).unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        let raw: String = connection
            .query_row(
                "SELECT settings_json FROM conversation_settings WHERE conversation_id='seed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let saved: Value = serde_json::from_str(&raw).unwrap();
        let fields = &saved["modelOverrides"]["configured:model"];
        if seed.is_null() {
            assert!(fields.get("seed").is_none());
        } else {
            assert_eq!(
                fields["seed"], seed,
                "Clear must replace rather than merge with old value"
            );
        }
    }
}

#[test]
fn session_patch_is_atomic_sparse_and_preserves_cross_client_settings() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("configuration.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    store.create_conversation(&CreateConversationInput{id:"c".into(),title:"Conversation".into(),revision:1,settings_json:Some(json!({"nativeOnly":{"keep":true},"modelOverrides":{"configured:a":{"temperature":0.7},"configured:b":{"seed":7}},"owner":"web","conversation":{"conversationId":"c"}}).to_string()),created_at_ms:1,updated_at_ms:1}).unwrap();
    let input=PatchConversationConfigurationInput{conversation_id:"c".into(),expected_revision:1,patch_json:json!({"permissionProfile":"cautious","modelOverrides":{"configured:a":{"temperature":0,"topP":0.2}}}).to_string().parse().unwrap(),updated_at_ms:2};
    let receipt: Value = committed(store
            .patch_conversation_configuration(&input)
            .unwrap());
    assert_eq!(receipt["configurationRevision"], 2);
    assert_eq!(receipt["modelOverrides"]["configured:a"]["temperature"], 0);
    assert_eq!(receipt["modelOverrides"]["configured:b"]["seed"], 7);
    let count = store.pending_sync_changes(100).unwrap().len();
    assert!(store
        .patch_conversation_configuration(&input)
        .unwrap().is_conflict());
    assert_eq!(store.pending_sync_changes(100).unwrap().len(), count);
    store.rename_conversation("c", "Renamed", None, 3).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    let raw: String = connection
        .query_row(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id='c'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let settings: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(settings["nativeOnly"]["keep"], true);
    assert_eq!(settings["permissionModeRawValue"], "cautious");
    assert_eq!(settings["modelOverrides"]["configured:a"]["temperature"], 0);
    let next=PatchConversationConfigurationInput{expected_revision:2,patch_json:json!({"permissionProfile":null,"modelOverrides":{"configured:a":{"temperature":null},"configured:b":null}}).to_string().parse().unwrap(),..input};
    let receipt: Value = committed(store
            .patch_conversation_configuration(&next)
            .unwrap());
    assert!(receipt["permissionProfile"].is_null());
    assert!(receipt["modelOverrides"]["configured:a"]
        .get("temperature")
        .is_none());
    assert_eq!(receipt["modelOverrides"]["configured:a"]["topP"], 0.2);
    assert!(receipt["modelOverrides"].get("configured:b").is_none());
}
