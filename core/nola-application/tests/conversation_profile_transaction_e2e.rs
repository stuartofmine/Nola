use nola_application::{execute_application_command, ApplicationCommand};
use nola_store::NolaStore;
use nola_store_port::conversation_profile::{CreateConversationInput, PatchConversationConfigurationInput};
use serde_json::json;

#[test]
fn configuration_compare_and_swap_is_independent_of_title_and_transcript() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("configuration.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_conversation(&CreateConversationInput { id:"c".into(),title:"Original".into(),revision:1,settings_json:Some("{}".into()),created_at_ms:1,updated_at_ms:1 }).unwrap();
    let command = |revision, value| ApplicationCommand::PatchConversationConfiguration { input:PatchConversationConfigurationInput { conversation_id:"c".into(),expected_revision:revision,patch_json:json!({"permissionProfile":value}).to_string().parse().unwrap(),updated_at_ms:2 } };
    store.rename_conversation("c", "New title", None, 2).unwrap();
    let committed = execute_application_command(&mut store, command(1,"cautious")).unwrap();
    assert_eq!(committed["status"],"committed");
    assert_eq!(committed["snapshot"]["configurationRevision"],2);
    let stale = execute_application_command(&mut store, command(1,"allow_all")).unwrap();
    assert_eq!(stale,json!({"status":"conflict","conversation_id":"c","expected_revision":1,"actual_revision":2}));
    drop(store);
    let store=NolaStore::open(&path).unwrap();
    let conversation=store.load_conversation("c").unwrap().unwrap();
    assert_eq!(conversation.title,"New title");
    assert_eq!(conversation.configuration_revision,2);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&conversation.settings_json.unwrap()).unwrap()["permissionModeRawValue"],"cautious");
}

#[test]
fn removed_whole_profile_commands_are_not_wire_aliases() {
    for command in ["update_conversation_settings","update_conversation_profile"] {
        assert!(serde_json::from_value::<ApplicationCommand>(json!({"type":command,"input":{"conversation_id":"c","settings_json":"{}","title":"stale","updated_at_ms":1}})).is_err());
    }
}

#[test]
fn active_policy_notification_is_owner_bound_atomic_with_configuration_cas() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("policy-cas.db");
    let mut store = NolaStore::open(&path).unwrap();
    for id in ["c", "other"] {
        store.create_conversation(&CreateConversationInput {
            id: id.into(), title: id.into(), revision: 1, settings_json: Some("{}".into()),
            created_at_ms: 1, updated_at_ms: 1,
        }).unwrap();
    }
    // Narrow fixture: active_run lookup and transactional notification, not a
    // claim that these synthetic rows are complete admitted executions.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('r','c','conversation','running',1,1,1),('other-r','other','conversation','running',1,1,1);").unwrap();
    let patch = |revision, mode| ApplicationCommand::PatchConversationConfiguration {
        input: PatchConversationConfigurationInput { conversation_id: "c".into(), expected_revision: revision,
            patch_json: json!({"permissionProfile":mode}).to_string().parse().unwrap(), updated_at_ms: 2 },
    };
    let committed = execute_application_command(&mut store, patch(1, "allow_all")).unwrap();
    assert_eq!(committed["snapshot"]["configurationRevision"], 2);
    let (owner, id, payload): (String,String,String) = db.query_row(
        "SELECT session_id,command_id,payload_json FROM agent_commands", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(owner,"c");
    assert_eq!(id,"permission-policy:r:2");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&payload).unwrap(),
        json!({"type":"permission_policy_available","policy":{"run_id":"r","configuration_revision":2,"profile":"allow_all"}}));
    assert_eq!(execute_application_command(&mut store, patch(1,"cautious")).unwrap()["status"],"conflict");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_commands",[],|row|row.get::<_,i64>(0)).unwrap(),1);
    db.execute_batch("CREATE TRIGGER reject_policy_notice BEFORE INSERT ON agent_commands
        BEGIN SELECT RAISE(ABORT,'policy notice fault'); END;").unwrap();
    assert!(execute_application_command(&mut store,patch(2,"cautious")).unwrap_err().contains("policy notice fault"));
    assert_eq!(store.load_conversation("c").unwrap().unwrap().configuration_revision,2);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_commands",[],|row|row.get::<_,i64>(0)).unwrap(),1);
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(store.load_conversation("c").unwrap().unwrap().configuration_revision,2);
}
