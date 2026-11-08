use nola_store::NolaStore;
use nola_store_port::conversation_profile::{CreateConversationInput, PatchConversationMetadataInput};
use serde_json::{json, Value};

#[test]
fn two_connections_reject_stale_metadata_and_preserve_unowned_fields() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("metadata.sqlite");
    let mut mac = NolaStore::open(&path).unwrap();
    mac.create_conversation(&CreateConversationInput {
        id: "shared".into(), title: "Shared".into(), revision: 1,
        settings_json: Some(json!({"isPinned": false, "permissionModeRawValue": "cautious",
            "knowledge": {"wikiMarkdown": "old"}, "conversation": {"keep": true}}).to_string()),
        created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    let mut web = NolaStore::open(&path).unwrap();
    let mut edit = PatchConversationMetadataInput {
        conversation_id: "shared".into(), expected_revision: 1,
        patch_json: json!({"clientNote": "新知识\n"}).to_string(),
        updated_at_ms: 2,
    };
    assert_eq!(web.patch_conversation_metadata(&edit).unwrap(), Some(2));
    edit.patch_json = json!({"isPinned": true}).to_string();
    assert_eq!(mac.patch_conversation_metadata(&edit).unwrap(), None);
    let read = |store: &NolaStore| -> Value {
        serde_json::from_str(&store.load_conversation_settings("shared").unwrap().unwrap()).unwrap()
    };
    assert_eq!(read(&mac)["isPinned"], false);
    edit.expected_revision = 2;
    assert_eq!(mac.patch_conversation_metadata(&edit).unwrap(), Some(3));
    drop(mac);
    let reopened = NolaStore::open(&path).unwrap();
    let settings = read(&reopened);
    assert_eq!(settings["isPinned"], true);
    assert_eq!(settings["permissionModeRawValue"],"cautious");
    assert_eq!(settings["knowledge"], json!({"wikiMarkdown": "old"}));
    assert_eq!(settings["clientNote"],"新知识\n");
    assert_eq!(settings["conversation"], json!({"keep": true}));
    edit.conversation_id = "deleted".into();
    assert_eq!(web.patch_conversation_metadata(&edit).unwrap(), None);
}

#[test]
fn invalid_metadata_does_not_mutate_data_or_revision() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(&root.path().join("invalid.sqlite")).unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "shared".into(), title: "Shared".into(), revision: 1,
        settings_json: Some("{}".into()), created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    let mut edit = PatchConversationMetadataInput {
        conversation_id: "shared".into(), expected_revision: 1,
        patch_json: "{}".into(), updated_at_ms: 2,
    };
    for raw in ["{}", "null", "[]", "false", "broken", "{\"modelOverrides\":null}", "{\"permissionModeRawValue\":\"allow_all\"}", "{\"sessionMemory\":\"text\"}"] {
        edit.patch_json = raw.into();
        assert!(store.patch_conversation_metadata(&edit).is_err());
        assert_eq!(store.load_conversation_settings("shared").unwrap().unwrap(), "{}");
    }
    edit.patch_json = json!({"isPinned": true}).to_string();
    edit.expected_revision = 0;
    assert!(store.patch_conversation_metadata(&edit).is_err());
    edit.expected_revision = 1;
    assert_eq!(store.patch_conversation_metadata(&edit).unwrap(), Some(2));
    assert!(serde_json::from_value::<PatchConversationMetadataInput>(json!({
        "conversation_id": "shared", "expected_revision": 2, "patch_json": "{}",
        "updated_at_ms": 3, "legacy": true
    })).is_err());
}
