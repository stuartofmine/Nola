// SPDX-License-Identifier: Apache-2.0
//! Same-conversation rewind, rollback, compacted history and retry regressions.
mod support;
use nola_application::{execute_application_command, ApplicationCommand};
use nola_domain::edited_resend::{PrepareInPlaceResend, PreparedInPlaceResend};
use nola_store::NolaStore;

fn seed(path: &std::path::Path) -> PrepareInPlaceResend {
    drop(NolaStore::open(path).unwrap());
    let db = rusqlite::Connection::open(path).unwrap();
    db.execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('history','History',1,1,1)", []).unwrap();
    for (ordinal, role, text) in [
        (1, "user", "retained question"),
        (2, "assistant", "retained answer"),
        (3, "user", "abandoned question"),
        (4, "assistant", "abandoned answer"),
        (5, "user", "abandoned followup"),
    ] {
        let id = format!("m{ordinal}");
        db.execute(
            "INSERT INTO messages VALUES(?1,'history',NULL,?2,'completed',?3,1,2)",
            rusqlite::params![id, role, ordinal],
        )
        .unwrap();
        db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms) VALUES(?1,?2,'text',1,'completed',?3,1,2)",rusqlite::params![format!("{id}:text"),id,text]).unwrap();
    }
    let store = NolaStore::open(path).unwrap();
    let projection = nola_application::load_conversation_projection_v1(&store, "history").unwrap();
    PrepareInPlaceResend {
        operation_id: uuid::Uuid::new_v4().to_string(),
        conversation_id: "history".into(),
        message_id: "m3".into(),
        expected_revision: 1,
        expected_message_revision: projection.messages[2].revision,
        edited_text: "edited question".into(),
        created_at_ms: 10,
    }
}
fn prepare(
    store: &mut NolaStore,
    input: &PrepareInPlaceResend,
) -> Result<PreparedInPlaceResend, String> {
    execute_application_command(
        store,
        ApplicationCommand::PrepareInPlaceResend {
            input: input.clone(),
        },
    )
    .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}
#[test]
fn rewinds_same_owner_and_cold_retry_never_truncates_new_output() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("test.db");
    let mut input = seed(&path);
    let mut store = NolaStore::open(&path).unwrap();
    use nola_agent_engine::agent::recovery::CheckpointReason;
    use nola_agent_engine::agent::{
        AgentCore, AgentCoreConfig, AgentItem, AgentRole, AgentSubmission, ItemId, SessionId,
    };
    let mut core = AgentCore::try_new(AgentCoreConfig {
        session_id: Some(SessionId::from_string("history".into())),
        initial_items: vec![AgentItem::Message {
            id: ItemId::from_string("old-summary".into()),
            role: AgentRole::Assistant,
            content: "abandoned compacted summary".into(),
        }],
        ..support::model_identity::config()
    })
    .unwrap();
    core.submit(AgentSubmission::RebaseConversation {
        items: vec![AgentItem::Message {
            id: ItemId::from_string("old-summary".into()),
            role: AgentRole::Assistant,
            content: "abandoned compacted summary".into(),
        }],
        item_ownership: Default::default(),
    })
    .unwrap();
    let checkpoint = core.into_checkpoint(CheckpointReason::Manual);
    store
        .save_checkpoint(&nola_store::SaveCheckpointInput {
            conversation_id: "history".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: checkpoint.last_event_cursor.sequence as i64,
            checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
            updated_at_ms: 2,
        })
        .unwrap();
    input.expected_message_revision =
        nola_application::load_conversation_projection_v1(&store, "history")
            .unwrap()
            .messages
            .iter()
            .find(|m| m.message_id.0 == "m3")
            .unwrap()
            .revision;
    let result = prepare(&mut store, &input).unwrap();
    let window = store
        .load_conversation_message_window("history", None, 20, 65536)
        .unwrap()
        .unwrap();
    assert_eq!(
        window
            .conversation
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m1", "m2"]
    );
    let checkpoint = store
        .load_checkpoint("history")
        .unwrap()
        .unwrap()
        .checkpoint_json;
    assert!(checkpoint.contains("retained question"));
    assert!(checkpoint.contains("retained answer"));
    assert!(!checkpoint.contains("abandoned"));
    assert!(
        !checkpoint.contains("edited question"),
        "new input is admitted separately"
    );
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "INSERT INTO messages VALUES('new-answer','history',NULL,'assistant','completed',6,11,12)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms) VALUES('new-answer:text','new-answer','text',1,'completed','new answer',11,12)",[]).unwrap();
    let mut store = NolaStore::open(&path).unwrap();
    assert_eq!(prepare(&mut store, &input).unwrap(), result);
    let window = store
        .load_conversation_message_window("history", None, 20, 65536)
        .unwrap()
        .unwrap();
    assert_eq!(window.total_message_count, 3);
    assert_eq!(
        window.conversation.messages.last().unwrap().id,
        "new-answer"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM conversations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        6,
        "original journal retained"
    );
    let mut conflict = input.clone();
    conflict.edited_text = "different edit".into();
    assert!(prepare(&mut store, &conflict).is_err());
}
#[test]
fn stale_witness_or_late_failure_leaves_history_and_checkpoint_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("test.db");
    let input = seed(&path);
    let mut store = NolaStore::open(&path).unwrap();
    let mut stale = input.clone();
    stale.expected_revision += 1;
    assert!(prepare(&mut store, &stale).is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_edit BEFORE INSERT ON product_settings WHEN NEW.key LIKE 'edited_resend/in_place/%' BEGIN SELECT RAISE(ABORT,'late fault'); END").unwrap();
    assert!(prepare(&mut store, &input).is_err());
    assert!(store.load_checkpoint("history").unwrap().is_none());
    assert_eq!(
        store
            .load_conversation_message_window("history", None, 20, 65536)
            .unwrap()
            .unwrap()
            .total_message_count,
        5
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM message_projection_events", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_edit").unwrap();
    assert!(prepare(&mut store, &input).is_ok());
}

#[test]
fn canonical_input_after_rewind_uses_raw_ledger_frontier() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("test.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_conversation(&nola_store_port::conversation_profile::CreateConversationInput {
        id:"history".into(),title:"History".into(),revision:1,settings_json:None,created_at_ms:1,updated_at_ms:1,
    }).unwrap();
    for i in 0..3 {
        execute_application_command(&mut store,ApplicationCommand::AdmitConversationUserMessage {
            input:nola_protocol::AdmitConversationUserMessage { conversation_id:"history".into(),
                message_id:format!("u{i}"),text:format!("original-{i}"),created_at_ms:i+2 },
        }).unwrap();
    }
    let projection=nola_application::load_conversation_projection_v1(&store,"history").unwrap();
    let input=PrepareInPlaceResend {operation_id:uuid::Uuid::new_v4().to_string(),conversation_id:"history".into(),
        message_id:"u1".into(),expected_revision:projection.revision as i64,
        expected_message_revision:projection.messages[1].revision,edited_text:"replacement".into(),created_at_ms:10};
    let prepared = prepare(&mut store,&input).unwrap();
    execute_application_command(&mut store,ApplicationCommand::AdmitConversationUserMessage {
        input:nola_protocol::AdmitConversationUserMessage { conversation_id:"history".into(),
            message_id:prepared.user_message_id.clone(),text:input.edited_text.clone(),created_at_ms:input.created_at_ms },
    }).unwrap();
    let visible=store.load_conversation_message_window("history",None,20,65536).unwrap().unwrap();
    assert_eq!(visible.total_message_count,2);
    assert_eq!(visible.conversation.messages.last().unwrap().ordinal,4);
    assert_eq!(prepare(&mut store,&input).unwrap(),prepared);
    assert_eq!(store.load_conversation_message_window("history",None,20,65536).unwrap().unwrap().total_message_count,2);
}

#[test]
fn opaque_message_metadata_does_not_make_disclosed_user_text_uneditable() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("ios.db")).unwrap();
    store.create_conversation(&nola_store_port::conversation_profile::CreateConversationInput {
        id:"ios".into(),title:"Metadata".into(),revision:1,settings_json:None,created_at_ms:1,updated_at_ms:1,
    }).unwrap();
    store.append_messages(&nola_store_port::transcript_write::AppendMessagesInput {
        conversation_id:"ios".into(),expected_message_count:0,updated_at_ms:2,
        messages:vec![nola_store_port::transcript_write::MessageInput {
            id:"user".into(),run_id:None,role:"user".into(),status:"completed".into(),ordinal:1,
            created_at_ms:2,completed_at_ms:Some(2),parts:vec![nola_domain::MessagePartInput {
                id:"user:text".into(),kind:"text".into(),ordinal:1,state:"completed".into(),
                content_text:"original text".into(),content_json:Some(r#"{"nola.canonicalMessageID":"user"}"#.into()),
                content_digest:None,created_at_ms:2,updated_at_ms:2,
            }],
        }],
    }).unwrap();
    let projection=nola_application::load_conversation_projection_v1(&store,"ios").unwrap();
    assert!(matches!(&projection.messages[0].parts[0], nola_domain::MessagePartView::Historical { .. }));
    let input=PrepareInPlaceResend {operation_id:uuid::Uuid::new_v4().to_string(),conversation_id:"ios".into(),
        message_id:"user".into(),expected_revision:projection.revision as i64,
        expected_message_revision:projection.messages[0].revision,edited_text:"replacement".into(),created_at_ms:3};
    prepare(&mut store,&input).unwrap();
    assert_eq!(store.load_conversation_message_window("ios",None,10,65536).unwrap().unwrap().total_message_count,0);
}
