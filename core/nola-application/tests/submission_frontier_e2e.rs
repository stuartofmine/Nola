// SPDX-License-Identifier: Apache-2.0
//! Actual SQLite ownership/frontier reads; no model or platform UI is simulated as evidence.
mod support;
use nola_application::ApplicationCommand;
use nola_domain::remote_submission::{RemoteSubmissionIdentity, SubmissionFrontier, SubmissionFrontierRequest};

fn request() -> SubmissionFrontierRequest {
    SubmissionFrontierRequest { expected_host_id: "host".into(), client_id: "client".into(),
        conversation_id: "conversation".into(), expected_run_id: None, assistant_message_id: None }
}
fn observe(runtime: &mut support::ApplicationRuntime, request: SubmissionFrontierRequest) -> Result<SubmissionFrontier, String> {
    serde_json::from_value(runtime.execute(ApplicationCommand::ObserveSubmissionFrontier {
        source_host_id: "host".into(), request })?).map_err(|error| error.to_string())
}

#[test]
fn new_frontier_is_read_only_and_existing_frontier_requires_exact_source_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frontier.sqlite");
    let mut runtime = support::open_runtime(&path).unwrap();
    let new = observe(&mut runtime, request()).unwrap();
    assert_eq!((new.configuration_revision, new.parent_message_id), (None, None));
    let mut wrong = request(); wrong.expected_host_id = "foreign".into();
    assert!(observe(&mut runtime, wrong).is_err());
    let mut half = request(); half.expected_run_id = Some("run".into());
    assert!(observe(&mut runtime, half).is_err());
    drop(runtime);
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let identity = RemoteSubmissionIdentity { source_host_id:"host".into(),client_id:"client".into(),
        conversation_id:"conversation".into(),submission_id:"run".into(),user_message_id:"user".into(),assistant_message_id:"assistant".into() };
    store.register_source_submission(&identity, &serde_json::json!({"input_items":[{"text":"actual source input"}]})).unwrap();
    store.begin_turn(&turn()).unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert!(observe(&mut runtime, request()).is_err(), "an existing conversation is not a new owner");
    let mut owned = request(); owned.expected_run_id = Some("run".into()); owned.assistant_message_id = Some("assistant".into());
    let snapshot = observe(&mut runtime, owned.clone()).unwrap();
    assert_eq!(snapshot.configuration_revision, Some(1));
    assert_eq!(snapshot.parent_message_id.as_deref(), Some("assistant"));
    let mut other = owned.clone(); other.client_id = "another-client".into();
    assert!(observe(&mut runtime, other).is_err());
    let mut other = owned.clone(); other.assistant_message_id = Some("another-assistant".into());
    assert!(observe(&mut runtime, other).is_err());
    assert_eq!(observe(&mut runtime, owned).unwrap(), snapshot);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(db.query_row::<i64,_,_>("SELECT COUNT(*) FROM runs", [], |row|row.get(0)).unwrap(), 1);
    assert_eq!(db.query_row::<i64,_,_>("SELECT COUNT(*) FROM messages", [], |row|row.get(0)).unwrap(), 2);
}

fn turn() -> nola_store_port::run_creation::BeginTurnInput {
    nola_store_port::run_creation::BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        conversation_id: "conversation".into(),
        conversation_title: "Remote".into(),
        user_message_id: "user".into(),
        user_part_id: "source-part".into(),
        user_text: "actual source input".into(),
        user_content_json: Some("{\"source\":true}".into()),
        additional_user_parts: vec![],
        assistant_message_id: "assistant".into(),
        assistant_part_id: "assistant:text".into(),
        run_id: "run".into(),
        context_checkpoint_id: None,
        created_at_ms: 100,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture".into(),
        outbox_effects: vec![],
        initial_assistant_part_mutations: vec![],
        assistant_turn_bindings: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
    }
}
