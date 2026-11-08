// SPDX-License-Identifier: Apache-2.0
//! Isolated SQLite and actual Core question reductions; no provider or user Store.
use super::*;
use nola_store::NolaStore;
use nola_store_port::conversation_profile::CreateConversationInput;

fn conversation(store: &mut NolaStore, id: &str, at: i64) {
    store.create_conversation(&CreateConversationInput { id: id.into(), title: id.into(), revision: 1,
        settings_json: None, created_at_ms: at, updated_at_ms: at }).unwrap();
}
fn query(limit: usize) -> TodayFactsQuery {
    TodayFactsQuery { conversation_offset: 0, conversation_limit: limit, queued_limit: 5 }
}
#[test]
fn old_inactive_parent_question_is_discovered_before_recent_page_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("today.db");
    let (_, original) = crate::user_input_wait_tests::fixture_with_read(&path, true, false);
    let mut store = NolaStore::open(&path).unwrap();
    conversation(&mut store, "parent", 1);
    for i in 0..205 { conversation(&mut store, &format!("recent-{i}"), 10_000+i); }
    let facts = load_today_facts(&mut store, query(1)).unwrap();
    assert_eq!(facts.recent_conversations.len(), 5);
    assert_eq!(facts.recent_conversations[0].conversation_id, "recent-204");
    assert!(facts.has_more_conversations);
    assert_eq!(facts.next_conversation_offset, Some(1));
    let parent = &facts.conversations[0];
    assert_eq!(parent.conversation_id, "parent");
    assert!(parent.needs_response);
    assert!(parent.active_run.is_none(), "Child questions do not manufacture a parent Run");
    assert_eq!(parent.owned_task_waits.len(), 1);
    let child = &parent.owned_task_waits[0];
    assert_eq!(child.parent_run_id, "parent-run");
    assert_eq!(child.task_id, "task");
    assert_eq!(child.child_conversation_id, original.conversation_id);
    assert_eq!(child.child_run_id, original.run_id);
    assert!(!child.admission_attempt_id.is_empty());
    assert_eq!(serde_json::to_value(&child.stored.wait).unwrap(), serde_json::to_value(&original.wait).unwrap());
    // The query does not answer, mutate, or acquire a task lease.
    assert_eq!(store.open_user_input_waits(&original.conversation_id, Some(&original.run_id)).unwrap().len(), 1);
}
#[test]
fn queued_instances_are_filtered_before_limit_and_share_the_conversation_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("today.db");
    let mut store = NolaStore::open(&path).unwrap();
    conversation(&mut store, "original", 1);
    let writer = rusqlite::Connection::open(&path).unwrap();
    for (id, status, at) in [("earliest","queued",1), ("later","queued",2), ("noise","reconciled",999)] {
        writer.execute("INSERT INTO automation_runs(id,automation_id,definition_revision,occurrence_key,trigger,status,scheduled_for_ms,created_at_ms,updated_at_ms)
            VALUES(?1,'automation',1,?1,'schedule',?2,?3,1,1)", rusqlite::params![id,status,at]).unwrap();
    }
    let input = TodayFactsQuery { queued_limit: 1, ..query(10) };
    let facts = store.with_consistent_read(|reader| {
        // Pin before the second connection commits. All subsequent facts must
        // remain on this read, including automation data from the narrow port.
        assert_eq!(reader.list_conversations().unwrap()[0].title, "original");
        writer.execute("UPDATE conversations SET title='changed' WHERE id='original'", []).unwrap();
        writer.execute("UPDATE automation_runs SET status='reconciled' WHERE id='earliest'", []).unwrap();
        read(reader, &input)
    }).unwrap();
    assert_eq!(facts.conversations[0].title, "original");
    assert_eq!(facts.queued_instances[0].occurrence_id, "earliest");
    assert!(facts.has_more_queued_instances);
    assert!(facts.queued_instances[0].title.is_none(), "Do not invent a missing admitted definition title");
    let refreshed = load_today_facts(&mut store, input).unwrap();
    assert_eq!(refreshed.conversations[0].title, "changed");
    assert_eq!(refreshed.queued_instances[0].occurrence_id, "later");
    assert!(!refreshed.has_more_queued_instances);
}
#[test]
fn invalid_page_bounds_are_rejected_without_creating_records() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("today.db")).unwrap();
    assert!(load_today_facts(&mut store, query(0)).is_err());
    assert!(load_today_facts(&mut store, TodayFactsQuery { queued_limit: 51, ..query(10) }).is_err());
    let facts = load_today_facts(&mut store, query(10)).unwrap();
    assert!(facts.conversations.is_empty());
    assert!(facts.queued_instances.is_empty());
}

#[test]
fn exact_open_question_witness_survives_snapshot_and_query_does_not_answer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("today.db");
    let (_, original) = crate::user_input_wait_tests::fixture_with_read(&path, false, false);
    let mut store = NolaStore::open(&path).unwrap();
    let facts = load_today_facts(&mut store, query(10)).unwrap();
    let fact = facts.conversations.iter().find(|row| row.conversation_id == original.conversation_id).unwrap();
    assert!(fact.needs_response);
    assert_eq!(fact.active_run.as_ref().unwrap().run_id, original.run_id);
    assert_eq!(fact.active_run.as_ref().unwrap().assistant_message_id, "assistant");
    assert_eq!(serde_json::to_value(&fact.open_waits[0].wait).unwrap(), serde_json::to_value(&original.wait).unwrap());
    assert_eq!(store.open_user_input_waits(&original.conversation_id, Some(&original.run_id)).unwrap().len(), 1);
}

#[test]
fn remote_mirror_questions_are_not_presented_as_local_authority() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("today.db");
    let (_, wait) = crate::user_input_wait_tests::fixture_with_read(&path, false, false);
    let mut store = NolaStore::open(&path).unwrap();
    let writer = rusqlite::Connection::open(&path).unwrap();
    // Seed source metadata to test exclusion; this does not test transport authentication:
    // the question and its original Run were produced by actual Core above.
    writer.execute("INSERT INTO remote_run_sources(run_id,source_host_id) VALUES(?1,'source')", [&wait.run_id]).unwrap();
    writer.execute("INSERT INTO remote_assistant_journal_sources(run_id,source_host_id,source_store_epoch,assistant_message_id)
        VALUES(?1,'source','epoch','assistant')", [&wait.run_id]).unwrap();
    let facts = load_today_facts(&mut store, query(10)).unwrap();
    assert!(facts.conversations.is_empty());
    assert_eq!(store.open_user_input_waits(&wait.conversation_id, Some(&wait.run_id)).unwrap().len(), 1,
        "Exclusion must not delete or resolve the mirror question");
}

#[test]
fn child_permission_alone_marks_an_inactive_parent_as_needing_response() {
    // Presentation policy only; runtime permission production is exercised by
    // existing exact-permission/recovery tests, not simulated by this fixture.
    let mut fact = TodayConversationFact {
        conversation_id: "parent".into(), title: "Parent".into(), revision: 1, updated_at_ms: 1,
        needs_response: false, active_run: None, permission_projection: None,
        open_waits: vec![], owned_task_waits: vec![], expert_contexts: vec![],
        owned_task_permissions: vec![TodayOwnedTaskPermission {
            task_id: "task".into(), parent_conversation_id: "parent".into(), parent_run_id: "parent-run".into(),
            child_conversation_id: "child".into(), child_run_id: "child-run".into(), admission_attempt_id: "attempt".into(),
            permission_projection: serde_json::json!({"conversation_id":"child","run_id":"child-run", "permissions":{"pending":[{"id":"request"}]}}),
        }],
    };
    fact.needs_response = fact.has_waiting_response();
    assert!(fact.needs_response);
    assert!(fact.active_run.is_none());
    let encoded = serde_json::to_value(&fact).unwrap();
    assert_eq!(encoded["owned_task_permissions"][0]["permission_projection"]["conversation_id"], "child");
    assert_ne!(encoded["owned_task_permissions"][0]["permission_projection"]["run_id"], "parent-run");
}

fn expert_question_fixture(path: &std::path::Path, associated: bool) -> (String, StoredUserInputWait) {
    let (_, wait) = crate::user_input_wait_tests::fixture_with_read(path, false, false);
    let mut store = NolaStore::open(path).unwrap();
    conversation(&mut store, "outer", 1);
    let room_id = uuid::Uuid::new_v4().to_string();
    let participant = nola_domain::agent_room::RoomParticipant {
        id: "expert-member".into(), agent_id: "expert".to_string().try_into().unwrap(), agent_revision: 3,
        display_name: "Expert full name".into(), description: "Fixture expert".into(),
        context_conversation_id: wait.conversation_id.clone(), active: false,
    };
    let room = nola_domain::agent_room::AgentRoom::new(room_id.clone(), "Experts".into(), vec![participant]).unwrap();
    store.put_product_setting(&format!("agent_chat_room/{room_id}"), &serde_json::to_string(&room).unwrap(), 1).unwrap();
    // Seed retained membership metadata at the read/import boundary. The real
    // question above comes from Core; this test does not claim to test joining.
    let writer = rusqlite::Connection::open(path).unwrap();
    writer.execute("INSERT INTO conversation_settings(conversation_id,settings_json,updated_at_ms) VALUES(?1,?2,1)
        ON CONFLICT(conversation_id) DO UPDATE SET settings_json=?2",rusqlite::params![wait.conversation_id,
        serde_json::json!({"agentRoomOwner":{"room_id":room_id,"participant_id":"expert-member","agent_id":"expert"}}).to_string()]).unwrap();
    if associated {
        let link = serde_json::json!({"conversation_id":"outer","room_id":room_id}).to_string();
        store.put_product_setting("agent_chat_conversation_room/outer", &link, 1).unwrap();
        store.put_product_setting(&format!("agent_chat_room_conversation/{room_id}"), &link, 1).unwrap();
    }
    (room_id, wait)
}
#[test]
fn expert_wait_maps_through_verified_bidirectional_association_and_retains_private_owner() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("today.db");
    let (_, original)=expert_question_fixture(&path,true);
    let mut store=NolaStore::open(&path).unwrap();
    let facts=load_today_facts(&mut store,query(10)).unwrap();
    assert!(!facts.needs_attention);
    assert!(!facts.conversations.iter().any(|fact|fact.conversation_id==original.conversation_id));
    let outer=facts.conversations.iter().find(|fact|fact.conversation_id=="outer").unwrap();
    assert!(outer.needs_response);
    assert!(outer.active_run.is_none());
    let expert=&outer.expert_contexts[0];
    assert_eq!(expert.canonical_conversation_id.as_deref(),Some("outer"));
    assert_eq!(expert.agent_revision,3);
    assert_eq!(expert.participant_id,"expert-member");
    assert_eq!(expert.context_conversation_id,original.conversation_id);
    assert_eq!(serde_json::to_value(&expert.fact.open_waits[0].wait).unwrap(),serde_json::to_value(&original.wait).unwrap());
}
#[test]
fn standalone_expert_work_is_explicit_attention_and_broken_reverse_index_is_an_error() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("today.db");
    let (room,original)=expert_question_fixture(&path,false);
    let mut store=NolaStore::open(&path).unwrap();
    let facts=load_today_facts(&mut store,query(10)).unwrap();
    assert!(facts.needs_attention);
    assert_eq!(facts.unrepresented_expert_contexts[0].context_conversation_id,original.conversation_id);
    assert!(!facts.conversations.iter().any(|fact|fact.conversation_id==original.conversation_id));
    let reverse=serde_json::json!({"conversation_id":"outer","room_id":room}).to_string();
    store.put_product_setting(&format!("agent_chat_room_conversation/{room}"),&reverse,1).unwrap();
    assert!(load_today_facts(&mut store,query(10)).is_err(),"A reverse-only index cannot manufacture a canonical owner");
}
#[test]
fn unknown_active_conversation_state_and_unproven_maintenance_are_not_silently_empty() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("today.db");
    let (_,wait)=crate::user_input_wait_tests::fixture_with_read(&path,false,false);
    let mut store=NolaStore::open(&path).unwrap();
    let writer=rusqlite::Connection::open(&path).unwrap();
    writer.execute("UPDATE runs SET status='invalid' WHERE id=?1",[wait.run_id]).unwrap();
    assert!(load_today_facts(&mut store,query(10)).is_err());
    let path=dir.path().join("maintenance.db");let mut store=NolaStore::open(&path).unwrap();
    conversation(&mut store,"background",1);
    let writer=rusqlite::Connection::open(&path).unwrap();
    writer.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('maintenance','background','maintenance','running',0,1,1)",[]).unwrap();
    assert!(load_today_facts(&mut store,query(10)).is_err(),
        "A bare maintenance header without a Core checkpoint is not proven background work");
}
