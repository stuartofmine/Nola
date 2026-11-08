// SPDX-License-Identifier: Apache-2.0
//! Store frontier rejection uses Core revisions, not durable commit counters.
use super::*;
use serde_json::json;

#[test]
fn permission_provenance_retains_declaration_identity_after_delivery_rebind() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open_archive_mode(&root.path().join("source.db"), false).unwrap();
    let tx = store.write_transaction().unwrap();
    tx.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms)
        VALUES('fixture-session','permission',1,1,1);
        INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('run','fixture-session','maintenance','running',1,1,1)").unwrap();
    let mut effect = OutboxEffectInput { effect_id:"effect".into(),request_id:"original".into(),
        effect_kind:"permission.request".into(),model_attempt_id:None,delegated_invocation_id:None,
        write:crate::outbox_fixture::declare("effect","original",json!({
            "type":"request_permission","allow_conversation":false,"permission_request_id":"ask",
            "capability_id":"bash","resource":"project","reason":"review","input":{"command":"nl file"}
        })) };
    crate::outbox_payload::write(&tx,"run",&effect,1).unwrap();
    let mut checkpoint=json!({"session_id":"fixture-session","revision":1,
        "state":{"session_id":"fixture-session","execution":{"permissions":{"pending":[]}},
        "host":{"pending_host_effects":{"effects":{"effect":{
            "effect_id":"effect","request_id":"original","session_id":"fixture-session",
            "turn_id":null,"host_id":"fixture-host","catalog_revision":1,"availability_epoch":1,
            "replay_safety":"read_only","request":{"payload":{
                "type":"request_permission","permission_request_id":"ask"}}}}}}}});
    tx.execute("INSERT INTO core_checkpoints VALUES('fixture-session',1,1,1)",[]).unwrap();
    tx.execute("INSERT INTO core_checkpoint_bodies VALUES('fixture-session',?1)",[checkpoint.to_string()]).unwrap();
    let mut request:StoredPermissionRequest=serde_json::from_value(json!({
        "closure":null,"request_id":"outer","task_id":"task","child_session_id":"fixture-session",
        "capability":"bash","input_json":"{\"command\":\"nl file\"}","scope_rule":"once",
        "status":"pending","decision":null,"custom_response":null,"created_at_ms":1,
        "resolved_at_ms":null,"core_permission_request_id":"ask","permission_instructions_json":"[]"
    })).unwrap();
    let original=capture(&tx,"run",&request).unwrap();
    let previous=crate::outbox_payload::identity(&tx,"effect").unwrap().unwrap();
    let mut binding=previous.binding.clone();binding.request_id="successor".into();
    effect.request_id="successor".into();
    effect.write=nola_store_port::run_effect::OutboxEffectWrite::Rebind {
        expected_request_id:previous.request_id,expected_attempt_id:previous.binding.attempt_id,binding };
    crate::outbox_payload::write(&tx,"run",&effect,2).unwrap();
    checkpoint["revision"]=json!(2);
    checkpoint["state"]["host"]["pending_host_effects"]["effects"]["effect"]["request_id"]=json!("successor");
    tx.execute("UPDATE core_checkpoints SET revision=2,event_cursor=2 WHERE conversation_id='fixture-session'",[]).unwrap();
    tx.execute("UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='fixture-session'",[checkpoint.to_string()]).unwrap();
    let resumed=capture(&tx,"run",&request).unwrap();
    assert_eq!(resumed.host_request_id,original.host_request_id);
    assert_eq!(resumed.permission,original.permission);
    assert_eq!(resumed.payload_digest,original.payload_digest);
    request.input_json="{}".into();
    assert!(capture(&tx,"run",&request).is_err(),"rebind cannot change the authorized arguments");
}

#[test]
fn stale_and_future_core_permission_sources_fail_before_settlement() {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE runs(id TEXT,conversation_id TEXT);
         CREATE TABLE core_checkpoints(conversation_id TEXT,revision INTEGER,event_cursor INTEGER);
         CREATE TABLE core_delivery_witnesses(conversation_id TEXT,checkpoint_revision INTEGER,event_cursor INTEGER,witness_json TEXT);
         INSERT INTO runs VALUES('run','conversation');
         INSERT INTO core_checkpoints VALUES('conversation',10,40);
         INSERT INTO core_delivery_witnesses VALUES('conversation',10,40,'{\"core_revision\":27}');"
    ).unwrap();
    for source_revision in [0, 26, 27, 30, u64::MAX] {
        let event = json!({"type":"permission_settled",
            "receipt":{"request_id":"ask","turn_id":"turn","call_id":"call",
                "capability_id":"read_file","resource":"fixture",
                "outcome":{"type":"authorization","decision":"allow_once"}},
            "core_envelope":{"session_id":"conversation","turn_id":"turn",
                "cursor":{"sequence":41},"revision":source_revision}});
        let input: nola_store_port::run_effect::CommitTransitionInput = serde_json::from_value(json!({
            "transition_id":"invalid-frontier","run_id":"run",
            "expected_checkpoint_revision":10,"checkpoint_revision":11,"checkpoint_event_cursor":44,
            "checkpoint":{"kind":"replay","delivery_witness_json":"{\"core_revision\":29}"},
            "host_event":null,"submission_reduction_json":null,
            "events":[{"event_id":"event","sequence":1,"created_at_ms":100,"event_kind":"permission_settled","payload_json":event.to_string()}],
            "assistant_turn_bindings":[],"part_mutations":[],"outbox_effects":[],
            "run_status":null,"assistant_status":null,"assistant_status_message_id":null,"committed_at_ms":100
        })).unwrap();
        let error = validate_settlements(&connection, &input).unwrap_err().to_string();
        assert!(error.contains("invalid permission settlement frontier"), "revision={source_revision}: {error}");
        assert_eq!(connection.query_row("SELECT revision FROM core_checkpoints", [], |r| r.get::<_, i64>(0)).unwrap(), 10);
    }
}
