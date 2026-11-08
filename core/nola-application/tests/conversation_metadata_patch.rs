use nola_application::{execute_application_command, ApplicationCommand};
use nola_store::NolaStore;
use nola_store_port::conversation_profile::CreateConversationInput;
use serde_json::{json, Value};

#[test]
fn metadata_query_returns_one_revision_without_message_bodies_or_writes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("metadata-read.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "shared".into(), title: "Not part of metadata response".into(), revision: 1,
        settings_json: Some("{\"isPinned\":false}".into()), created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute("INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms) VALUES('message','shared','user','completed',1,1)", []).unwrap();
    connection.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms) VALUES('part','message','text',1,'completed',?1,1,1)", ["PRIVATE TRANSCRIPT ".repeat(100_000)]).unwrap();
    let command: ApplicationCommand = serde_json::from_value(json!({
        "type":"load_conversation_metadata", "conversation_id":"shared"
    })).unwrap();
    assert!(command.is_read_only_query());
    let metadata = execute_application_command(&mut store, command).unwrap();
    assert_eq!(metadata, json!({"id":"shared", "revision":1, "configuration_revision":1, "settings_json":"{\"isPinned\":false}"}));
    assert_eq!(execute_application_command(&mut store, ApplicationCommand::LoadConversationMetadata {
        conversation_id: "missing".into()
    }).unwrap(), Value::Null);
    assert_eq!(connection.query_row("SELECT revision FROM conversations WHERE id='shared'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(connection.query_row("SELECT length(content_text) FROM message_parts WHERE id='part'", [], |row| row.get::<_, i64>(0)).unwrap(), 1_900_000);
}

#[test]
fn metadata_wire_command_is_a_revision_checked_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(&root.path().join("metadata.sqlite")).unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "shared".into(), title: "Shared".into(), revision: 1,
        settings_json: Some(json!({"knowledge": {"wikiMarkdown": "keep"}}).to_string()),
        created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    let wire = json!({"type": "patch_conversation_metadata", "input": {
        "conversation_id": "shared", "expected_revision": 1,
        "patch_json": "{\"isPinned\":true}", "updated_at_ms": 2
    }});
    let command: ApplicationCommand = serde_json::from_value(wire.clone()).unwrap();
    assert!(!command.is_read_only_query());
    assert_eq!(execute_application_command(&mut store, command).unwrap(), json!(2));
    let stale = serde_json::from_value(wire).unwrap();
    assert_eq!(execute_application_command(&mut store, stale).unwrap(), Value::Null);
    let saved: Value = serde_json::from_str(&store.load_conversation_settings("shared").unwrap().unwrap()).unwrap();
    assert_eq!(saved["knowledge"]["wikiMarkdown"], "keep");
    assert_eq!(saved["isPinned"], true);
    for seed in [json!(0), Value::Null, json!({"kind":"value", "value":"01"})] {
        let invalid = serde_json::from_value(json!({"type": "patch_conversation_metadata", "input": {
            "conversation_id": "shared", "expected_revision": 2,
            "patch_json": json!({"modelOverrides":{"configured:model":{"seed":seed}}}).to_string(),
            "updated_at_ms": 3
        }})).unwrap();
        assert!(execute_application_command(&mut store, invalid).is_err());
    }
    let remove = serde_json::from_value(json!({"type": "patch_conversation_metadata", "input": {
        "conversation_id": "shared", "expected_revision": 2,
        "patch_json": "{\"modelOverrides\":null}", "updated_at_ms": 3
    }})).unwrap();
    assert!(execute_application_command(&mut store, remove).is_err());
}
