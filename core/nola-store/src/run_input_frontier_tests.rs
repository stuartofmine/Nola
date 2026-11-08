// SPDX-License-Identifier: Apache-2.0
//! Real Store admission/transition transactions in isolated databases.
use super::*;
use crate::NolaStore;
use nola_store_port::run_creation::InitialCoreRunEvent;
use nola_store_port::run_effect::{CheckpointWrite, CommitTransitionInput, RunEventInput};
use serde_json::{json, Value};

fn accepted(id: &str, sequence: u64, next: bool) -> Value {
    json!({"type":"input_accepted","admission":{"admission_id":id,"submission_id":format!("submission-{id}"),
        "accepted_sequence":sequence,"origin":"interactive_user","delivery":if next {"next_turn"} else {"current_turn"},
        "anchor_turn_id":if next {Some("run")} else {None},"payload_digest":"a".repeat(64),"item_ids":[format!("item-{id}")]}})
}
fn applied(turn: &str, ids: &[(&str, u64)]) -> Value {
    json!({"type":"input_applied","turn_id":turn,"applications":ids.iter().map(|(id,sequence)|
        json!({"admission_id":id,"destination_turn_id":turn,"applied_sequence":sequence})).collect::<Vec<_>>()})
}
fn initial(payload: Value, cursor: u64) -> InitialCoreRunEvent {
    InitialCoreRunEvent {
        core_session_id: "core".into(),
        core_turn_id: Some("run".into()),
        core_revision: 10,
        core_cursor: cursor,
        event_kind: payload["type"].as_str().unwrap().into(),
        payload_json: payload.to_string(),
    }
}
fn begin(path: &std::path::Path, events: Vec<InitialCoreRunEvent>) -> NolaStore {
    let mut store = NolaStore::open(path).unwrap();
    store
        .begin_turn(&crate::input_facts::tests::input(events))
        .unwrap();
    store
}
fn transition(events: Vec<Value>) -> CommitTransitionInput {
    let events:Vec<_>=events.into_iter().enumerate().map(|(index,mut payload)| {
        let cursor=21+index as u64;
        payload["core_envelope"]=json!({"session_id":"core","turn_id":"second-turn","revision":11,"cursor":{"sequence":cursor}});
        RunEventInput {event_id:format!("core:run:{cursor}"),sequence:3+index as i64,event_kind:payload["type"].as_str().unwrap().into(),payload_json:payload.to_string(),created_at_ms:2}
    }).collect();
    let cursor = 20 + events.len() as i64;
    CommitTransitionInput {transition_id:"second-turn-input".into(),run_id:"run".into(),expected_checkpoint_revision:1,checkpoint_revision:2,
        checkpoint_event_cursor:cursor,checkpoint:CheckpointWrite::Baseline{checkpoint_json:json!({"session_id":"core","revision":11,"last_event_cursor":{"sequence":cursor},"state":{"session_id":"core"}}).to_string()},
        submission_reduction_json:None,host_event:None,events,ensured_messages:vec![],
        assistant_turn_bindings:vec![nola_domain::AssistantTurnBinding::TurnStarted{turn_id:"second-turn".into(),assistant_message_id:"assistant".into()}],
        part_message_owners:Default::default(),part_mutations:vec![],outbox_effects:vec![],model_attempt_mutations:vec![],
        delegated_invocation_mutations:vec![],delegated_permission_mutations:vec![],effect_delivery_attempt_mutations:vec![],
        acknowledged_effect_ids:vec![],memory_usage_entries:vec![],run_status:None,assistant_status:None,assistant_status_message_id:None,committed_at_ms:2}
}

#[test]
fn remote_conversation_retirement_cannot_bypass_active_run_protection() {
    for deleted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync-retirement.sqlite");
        let mut store = begin(&path, vec![
            initial(accepted("a", 1, false), 1),
            initial(applied("run", &[("a", 1)]), 2),
        ]);
        let id: String = store.connection.query_row(
            "SELECT conversation_id FROM runs WHERE id='run'", [], |row| row.get(0),
        ).unwrap();
        let before = serde_json::to_value(store.load_conversation(&id).unwrap().unwrap()).unwrap();
        let prefix = read_prefix(&store.connection, "run", 20).unwrap().unwrap();
        let mut replacement = before.clone();
        replacement["revision"] = json!(1000);
        let page = crate::ApplyRemoteSyncPageInput {
            replica_id: "retirement-replica".into(), next_cursor: 1, applied_at_ms: 100,
            changes: vec![crate::RemoteSyncChangeInput {
                cursor: 1, operation_id: "retire-current".into(), entity_type: "conversation".into(),
                entity_id: id.clone(), revision: 1000, deleted,
                payload_json: if deleted { "null".into() } else { replacement.to_string() },
                created_at_ms: 100,
            }],
        };
        let error = store.apply_remote_sync_page(&page).unwrap_err();
        assert!(error.to_string().contains("active"), "{error}");
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        assert_eq!(serde_json::to_value(store.load_conversation(&id).unwrap().unwrap()).unwrap(), before);
        let after_prefix = read_prefix(&store.connection, "run", 20).unwrap().unwrap();
        assert_eq!((after_prefix.run_sequence, after_prefix.core_cursor, after_prefix.chain_digest, after_prefix.provenance),
            (prefix.run_sequence, prefix.core_cursor, prefix.chain_digest, prefix.provenance));
        assert!(store.sync_cursor("retirement-replica").unwrap().is_none());
        // Reusing exactly the rejected page proves no successful receipt or
        // cursor was retained. Full Core termination is covered by App E2E.
        store.connection.execute("UPDATE runs SET status='sealed' WHERE id='run'", []).unwrap();
        store.apply_remote_sync_page(&page).unwrap();
        assert_eq!(store.sync_cursor("retirement-replica").unwrap().unwrap().cursor, 1);
        assert_eq!(store.load_conversation(&id).unwrap().is_none(), deleted);
    }
}

#[test]
fn input_facts_are_immutable_until_the_owning_conversation_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delete-owner.sqlite");
    let mut store = begin(&path, vec![
        initial(accepted("a", 1, false), 1),
        initial(applied("run", &[("a", 1)]), 2),
    ]);
    let conversation: String = store.connection.query_row(
        "SELECT conversation_id FROM runs WHERE id='run'", [], |row| row.get(0),
    ).unwrap();
    assert!(store.connection.execute("DELETE FROM run_input_applications", []).is_err());
    assert!(store.connection.execute("UPDATE run_input_applications SET run_sequence=2", []).is_err());
    assert!(store.delete_conversation(&conversation).is_err());
    // This Store fixture isolates the terminal deletion boundary; the
    // Application task binding E2E reaches terminal through the real reducer.
    store.connection.execute("UPDATE runs SET status='sealed' WHERE id='run'", []).unwrap();
    assert!(store.delete_conversation(&conversation).unwrap());
    for table in ["input_admissions", "input_turn_applications", "run_input_applications"] {
        let count: i64 = store.connection.query_row(
            &format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 0, "orphan facts in {table}");
    }
    assert!(store.connection.prepare("PRAGMA foreign_key_check").unwrap()
        .query([]).unwrap().next().unwrap().is_none());
}

#[test]
fn two_turn_local_first_inputs_form_one_dense_run_prefix_and_replay_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two-turns.sqlite");
    let mut store = begin(
        &path,
        vec![
            initial(accepted("a", 1, false), 1),
            initial(applied("run", &[("a", 1)]), 2),
        ],
    );
    assert_eq!(
        read_prefix(&store.connection, "run", 20)
            .unwrap()
            .unwrap()
            .run_sequence,
        1
    );
    let next = transition(vec![
        json!({"type":"turn_started","turn_id":"second-turn","assistant_message_id":"assistant"}),
        accepted("b", 2, false),
        applied("second-turn", &[("b", 1)]),
    ]);
    store.commit_transition(&next).unwrap();
    store.commit_transition(&next).unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let prefix = read_prefix(&store.connection, "run", 23).unwrap().unwrap();
    assert_eq!(prefix.run_sequence, 2);
    assert_eq!(prefix.core_cursor, 23);
    assert_eq!(prefix.provenance, "current_frontier");
    let rows:Vec<(i64,String,i64)>=store.connection.prepare("SELECT run_sequence,destination_turn_id,turn_applied_sequence FROM run_input_applications ORDER BY run_sequence").unwrap()
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(
        rows,
        vec![(1, "run".into(), 1), (2, "second-turn".into(), 1)]
    );
    assert_eq!(
        read_prefix(&store.connection, "run", 2)
            .unwrap()
            .unwrap()
            .run_sequence,
        1
    );
}

#[test]
fn one_event_preserves_application_order_and_queued_next_input_does_not_count() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch.sqlite");
    let store = begin(
        &path,
        vec![
            initial(accepted("a", 1, false), 1),
            initial(accepted("b", 2, false), 2),
            initial(accepted("next", 3, true), 3),
            initial(applied("run", &[("b", 1), ("a", 2)]), 4),
        ],
    );
    let rows:Vec<(String,i64,i64)>=store.connection.prepare("SELECT admission_id,run_sequence,event_application_index FROM run_input_applications ORDER BY run_sequence").unwrap()
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(rows, vec![("b".into(), 1, 0), ("a".into(), 2, 1)]);
    assert_eq!(
        read_prefix(&store.connection, "run", 20)
            .unwrap()
            .unwrap()
            .run_sequence,
        2
    );
    assert!(read_prefix(&store.connection, "run", 3).unwrap().is_none());
}

#[test]
fn run_prefix_rejects_damaged_applied_and_admission_source_without_erasing_evidence() {
    for source in ["applied", "accepted"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.sqlite");
        let store = begin(
            &path,
            vec![
                initial(accepted("a", 1, false), 1),
                initial(applied("run", &[("a", 1)]), 2),
            ],
        );
        read_prefix(&store.connection, "run", 20).unwrap().unwrap();
        drop(store);
        let sql = rusqlite::Connection::open(&path).unwrap();
        if source == "applied" {
            sql.execute("UPDATE run_events SET payload_json=json_set(payload_json,'$.applications[0].applied_sequence',7) WHERE event_id='core:run:2'",[]).unwrap();
        } else {
            sql.execute("UPDATE run_events SET payload_json=json_set(payload_json,'$.admission.payload_digest',?1) WHERE event_id='core:run:1'",["b".repeat(64)]).unwrap();
        }
        drop(sql);
        let store = NolaStore::open(&path).unwrap();
        assert!(
            read_prefix(&store.connection, "run", 20).is_err(),
            "{source}"
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM run_input_applications", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn decision_audit_checks_every_original_application_and_both_indexes() {
    for damage in ["none", "missing_prefix", "missing_application", "extra_raw_event", "changed_old_source"] {
        let dir=tempfile::tempdir().unwrap();
        let path=dir.path().join("decision-audit.sqlite");
        let mut store=begin(&path,vec![
            initial(accepted("a",1,false),1),initial(applied("run",&[("a",1)]),2),
        ]);
        store.commit_transition(&transition(vec![
            json!({"type":"turn_started","turn_id":"second-turn","assistant_message_id":"assistant"}),
            accepted("b",2,false),applied("second-turn",&[("b",1)]),
        ])).unwrap();
        let conversation:String=store.connection.query_row("SELECT conversation_id FROM runs WHERE id='run'",[],|r|r.get(0)).unwrap();
        let full=audit_for_decision(&store.connection,&conversation,"run","core",11,24).unwrap();
        assert_eq!(full.commitment.watermark.0,2);
        assert_eq!(full.applied[0].applied_sequence.0,1);
        assert_eq!(full.applied[1].applied_sequence.0,1);
        let old=audit_for_decision(&store.connection,&conversation,"run","core",10,3).unwrap();
        assert_eq!(old.commitment.watermark.0,1);
        match damage {
            "none" => continue,
            "missing_prefix" => {
                let triggers:Vec<String>=store.connection.prepare(
                    "SELECT name FROM sqlite_master WHERE type='trigger' AND tbl_name='run_input_applications'"
                ).unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
                for trigger in triggers { store.connection.execute_batch(&format!("DROP TRIGGER \"{trigger}\"")).unwrap(); }
                store.connection.execute("DELETE FROM run_input_applications WHERE run_sequence=1",[]).unwrap();
            }
            "missing_application" => {
                assert!(store.connection.execute("DELETE FROM input_turn_applications WHERE admission_id='a'",[]).is_err(),
                    "production immutable triggers reject corruption before it reaches the audit");
                let triggers:Vec<String>=store.connection.prepare(
                    "SELECT name FROM sqlite_master WHERE type='trigger' AND tbl_name IN ('input_turn_applications','run_input_applications')"
                ).unwrap().query_map([],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
                for trigger in triggers { store.connection.execute_batch(&format!("DROP TRIGGER \"{trigger}\"")).unwrap(); }
                store.connection.execute("DELETE FROM input_turn_applications WHERE admission_id='a'",[]).unwrap();
            }
            "extra_raw_event" => {
                store.connection.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms)
                    SELECT 'extra-source',run_id,99,event_kind,payload_json,created_at_ms FROM run_events WHERE event_id='core:run:2'",[]).unwrap();
            }
            _ => {
                store.connection.execute("UPDATE run_events SET payload_json=json_set(payload_json,'$.applications[0].admission_id','unknown')
                    WHERE event_id='core:run:2'",[]).unwrap();
            }
        }
        assert!(audit_for_decision(&store.connection,&conversation,"run","core",11,24).is_err(),"{damage}");
    }
}



