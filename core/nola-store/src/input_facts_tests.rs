// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::{BeginTurnInput, NolaStore};
use nola_store_port::run_creation::InitialCoreRunEvent;
use serde_json::json;

fn accepted(id: &str, sequence: u64) -> Value {
    json!({"type":"input_accepted","admission":{
        "admission_id":id,"submission_id":format!("submission-{id}"),"accepted_sequence":sequence,
        "origin":"interactive_user","delivery":"current_turn","anchor_turn_id":null,
        "payload_digest":"a".repeat(64),"item_ids":[format!("item-{id}")]}})
}
fn applied(id: &str, sequence: u64) -> Value {
    json!({"type":"input_applied","turn_id":"run","applications":[{
        "admission_id":id,"destination_turn_id":"run","applied_sequence":sequence}]})
}
fn initial(payload: Value, cursor: u64) -> InitialCoreRunEvent {
    InitialCoreRunEvent {
        core_session_id: "core".into(),
        core_turn_id: Some("run".into()),
        core_cursor: cursor,
        core_revision: 10,
        event_kind: payload["type"].as_str().unwrap().into(),
        payload_json: payload.to_string(),
    }
}
pub(crate) fn input(events: Vec<InitialCoreRunEvent>) -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events:events,initial_assistant_part_mutations:vec![],
        conversation_id:"conversation".into(),conversation_title:"Input".into(),
        user_message_id:"user".into(),user_part_id:"user-part".into(),user_text:"Body".into(),user_content_json:None,
        additional_user_parts:vec![],assistant_message_id:"assistant".into(),assistant_part_id:"part".into(),
        assistant_turn_bindings:vec![nola_domain::AssistantTurnBinding::Admission {turn_id:"run".into(),assistant_message_id:"assistant".into()}],
        run_id:"run".into(),context_checkpoint_id:None,checkpoint_revision:1,checkpoint_event_cursor:20,
        checkpoint_json:json!({"session_id":"core","revision":10,"last_event_cursor":{"sequence":20},"state":{"session_id":"core"}}).to_string(),
        invocation_config_json:"{}".into(),invocation_config_digest:"config".into(),outbox_effects:vec![],
        model_attempt_mutations:vec![],delegated_invocation_mutations:vec![],delegated_permission_mutations:vec![],memory_usage_entries:vec![],created_at_ms:1,
    }
}
fn stage(store: &NolaStore, id: &str, payload: Value, cursor: u64) -> StoreResult<()> {
    let mut payload = payload;
    payload["core_envelope"] =
        json!({"session_id":"core","turn_id":"run","revision":10,"cursor":{"sequence":cursor}});
    store.connection.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES(?1,'run',?2,?3,?4,1)",
        params![id,100+cursor as i64,payload["type"].as_str().unwrap(),payload.to_string()])?;
    index_event(&store.connection, id)
}

#[test]
fn shared_event_identity_rejects_extra_fields_before_indexing() {
    for extra_cursor_field in [false, true] {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.begin_turn(&input(vec![])).unwrap();
        let mut payload = accepted("a", 1);
        payload["core_envelope"] = json!({
            "session_id":"core","turn_id":"run","revision":10,"cursor":{"sequence":1}
        });
        if extra_cursor_field {
            payload["core_envelope"]["cursor"]["unverified"] = json!(true);
        } else {
            payload["core_envelope"]["unverified"] = json!(true);
        }
        store.connection.execute(
            "INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms) VALUES('bad-source','run',101,'input_accepted',?1,1)",
            params![payload.to_string()],
        ).unwrap();
        assert!(index_event(&store.connection, "bad-source").is_err());
        let count: i64 = store.connection.query_row(
            "SELECT COUNT(*) FROM input_admissions", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }
}

#[test]
fn initial_indices_are_atomic_and_survive_reopen_without_body_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("indices.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .begin_turn(&input(vec![
            initial(accepted("a", 1), 1),
            initial(applied("a", 1), 2),
        ]))
        .unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let row:(String,String,i64,String)=store.connection.query_row(
        "SELECT a.submission_id,p.destination_turn_id,p.turn_applied_sequence,p.provenance
         FROM input_admissions a JOIN input_turn_applications p USING(core_session_id,admission_id)",[],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        row,
        (
            "submission-a".into(),
            "run".into(),
            1,
            "current_frontier".into()
        )
    );
    let columns: Vec<String> = store
        .connection
        .prepare("PRAGMA table_info(input_admissions)")
        .unwrap()
        .query_map([], |r| r.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!columns
        .iter()
        .any(|c| c.contains("body") || c.contains("items_json")));
}

#[test]
fn rejected_initial_facts_roll_back_run_checkpoint_events_and_indices() {
    for case in [
        "digest",
        "items",
        "missing_admission",
        "sequence_gap",
        "accepted_gap",
        "session",
        "revision",
        "cursor",
        "oversized",
    ] {
        let mut store = NolaStore::open(":memory:").unwrap();
        let mut a = accepted("a", 1);
        if case == "accepted_gap" {
            a["admission"]["accepted_sequence"] = json!(2);
        }
        if case == "digest" {
            a["admission"]["payload_digest"] = json!("bad");
        }
        if case == "items" {
            a["admission"]["item_ids"] = json!(["same", "same"]);
        }
        if case == "oversized" {
            a["admission"]["item_ids"] = json!((0..300)
                .map(|i| format!("{i}-{}", "x".repeat(64)))
                .collect::<Vec<_>>());
        }
        let mut events = vec![
            initial(a, 1),
            initial(
                applied(
                    if case == "missing_admission" {
                        "absent"
                    } else {
                        "a"
                    },
                    if case == "sequence_gap" { 2 } else { 1 },
                ),
                2,
            ),
        ];
        if case == "session" {
            events[0].core_session_id = "wrong".into();
        }
        if case == "revision" {
            events[0].core_revision = 11;
        }
        if case == "cursor" {
            events[0].core_cursor = 21;
        }
        assert!(store.begin_turn(&input(events)).is_err(), "{case}");
        for table in [
            "runs",
            "core_checkpoints",
            "run_events",
            "input_admissions",
            "input_turn_applications",
        ] {
            let count: i64 = store
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "{case}: {table}");
        }
        assert!(store.connection.is_autocommit());
    }
}

#[test]
fn queued_withdrawn_and_conflicting_facts_preserve_original_source_on_rollback() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store
        .begin_turn(&input(vec![initial(accepted("a", 1), 1)]))
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    stage(
        &store,
        "withdraw",
        json!({"type":"input_withdrawn","turn_id":"run","admission_ids":["a"],"reason":"cancel"}),
        3,
    )
    .unwrap();
    store.commit_write_unit_of_work().unwrap();
    for (id, payload) in [("apply", applied("a", 1)), ("replace", accepted("a", 2))] {
        store.begin_write_unit_of_work().unwrap();
        assert!(stage(&store, id, payload, 4).is_err());
        store.rollback_write_unit_of_work().unwrap();
    }
    let counts:(i64,i64,i64)=store.connection.query_row("SELECT (SELECT COUNT(*) FROM input_admissions),(SELECT COUNT(*) FROM input_turn_applications),(SELECT COUNT(*) FROM input_dispositions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(counts, (1, 0, 1));
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM run_events WHERE event_id IN ('apply','replace')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn current_frontier_rejects_forged_core_revision_even_with_real_event_id() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.begin_turn(&input(vec![])).unwrap();
    store.begin_write_unit_of_work().unwrap();
    stage(&store, "accepted", accepted("a", 1), 1).unwrap();
    store.connection.execute("UPDATE run_events SET payload_json=json_set(payload_json,'$.core_envelope.revision',11) WHERE event_id='accepted'",[]).unwrap();
    assert!(index_event(&store.connection, "accepted").is_err());
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .connection
            .query_row("SELECT COUNT(*) FROM input_admissions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn repeated_source_event_cannot_change_digest_or_item_identity() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store
        .begin_turn(&input(vec![initial(accepted("a", 1), 1)]))
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    index_event(&store.connection, "core:run:1").unwrap();
    store.connection.execute("UPDATE run_events SET payload_json=json_set(payload_json,'$.admission.item_ids',json('[\"different\"]')) WHERE event_id='core:run:1'",[]).unwrap();
    assert!(index_event(&store.connection, "core:run:1").is_err());
    store.rollback_write_unit_of_work().unwrap();
}

