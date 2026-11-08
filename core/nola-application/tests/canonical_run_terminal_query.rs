use nola_application::{execute_application_command, ApplicationCommand};
use nola_store::NolaStore;

#[test]
fn terminal_query_is_read_only_and_unknown_run_has_no_evidence() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let command: ApplicationCommand = serde_json::from_value(serde_json::json!({
        "type":"canonical_run_terminal", "conversation_id":"conversation", "run_id":"run"
    })).unwrap();
    assert!(command.is_read_only_query());
    assert!(execute_application_command(&mut store, command).unwrap().is_null());
}
