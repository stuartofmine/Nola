// SPDX-License-Identifier: Apache-2.0
//! Current-format transactional replay-budget tests.
use super::*;
use nola_store_port::host_replay::{
    HostReplayReduction, HOST_REPLAY_MAX_BYTES, HOST_REPLAY_MAX_RECORDS, HOST_REPLAY_CHECKPOINT_RECORDS,
};
use nola_store_port::run_creation::{BeginTurnInput, RunCreationRepository};
use nola_store_port::run_effect::{
    CheckpointWrite, CommitTransitionInput, HostEventInput, RunEffectRepository,
};

fn begin(store: &mut NolaStore, id: &str) {
    store.begin_write_unit_of_work().unwrap();
    RunCreationRepository::begin_turn(
        store,
        &BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: vec![],
            initial_assistant_part_mutations: vec![],
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
            conversation_id: id.into(),
            conversation_title: "fixture".into(),
            user_message_id: format!("{id}:u"),
            user_part_id: format!("{id}:u:text"),
            user_text: "fixture".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: format!("{id}:a"),
            assistant_part_id: format!("{id}:a:text"),
            assistant_turn_bindings: vec![],
            run_id: id.into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: format!("{id}:config"),
            outbox_effects: vec![],
            created_at_ms: 1,
        },
    )
    .unwrap();
    store.commit_write_unit_of_work().unwrap();
    baseline(store, id, 1);
}

fn baseline(store: &mut NolaStore, id: &str, previous: i64) {
    store.begin_write_unit_of_work().unwrap();
    RunEffectRepository::commit_transition(
        store,
        &CommitTransitionInput {
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            effect_delivery_attempt_mutations: vec![],
            model_attempt_mutations: vec![],
            transition_id: format!("{id}:baseline:{}", previous + 1),
            run_id: id.into(),
            expected_checkpoint_revision: previous,
            checkpoint_revision: previous + 1,
            checkpoint_event_cursor: 0,
            checkpoint: CheckpointWrite::Baseline {
                checkpoint_json: "{}".into(),
            },
            submission_reduction_json: None, host_event: None,
            events: vec![],
            ensured_messages: vec![],
            assistant_turn_bindings: vec![],
            part_message_owners: Default::default(),
            part_mutations: vec![],
            outbox_effects: vec![],
            acknowledged_effect_ids: vec![],
            run_status: None,
            assistant_status: None,
            assistant_status_message_id: None,
            committed_at_ms: previous + 10,
        },
    )
    .unwrap();
    store.commit_write_unit_of_work().unwrap();
}

fn event(id: &str, sequence: i64, reduction: &str) -> HostEventInput {
    let fact: HostReplayReduction = serde_json::from_str(reduction).unwrap();
    HostEventInput {
        event_id: format!("{id}:{sequence}"),
        effect_id: format!("{id}:effect"),
        request_id: format!("{id}:request"),
        sequence,
        payload_json: fact.normalized_response().get().into(),
        reduction_json: Some(reduction.into()),
        received_at_ms: 73,
    }
}




fn response(id: &str, sequence: i64, padding: usize) -> serde_json::Value {
    serde_json::json!({"request_id":format!("{id}:request"),"effect_id":format!("{id}:effect"),
        "host_id":"host","lease_id":null,"session_id":id,"turn_id":"turn",
        "catalog_revision":1,"availability_epoch":1,"sequence":sequence,
        "attempt_id":null,"delegated_invocation_id":null,"delivery":null,
        "payload":{"type":"model_delta","channel":"text","part_id":"part","delta":"x".repeat(padding)}})
}


fn replay_json(id: &str, sequence: i64, padding: usize) -> String {
    serde_json::to_string(&HostReplayReduction::Replayable {
        protocol_version: 1,
        decision_time_ms: 73,
        normalized_response: serde_json::value::RawValue::from_string(
            response(id, sequence, padding).to_string(),
        )
        .unwrap(),
        binding: None,
    })
    .unwrap()
}

#[test]
fn replay_byte_and_record_frontiers_are_bounded_atomic_and_reset_by_a_baseline() {
    let mut store = NolaStore::open(":memory:").unwrap();
    begin(&mut store, "records");
    begin(&mut store, "bytes");
    let tiny = replay_json("records", HOST_REPLAY_CHECKPOINT_RECORDS as i64, 0);
    let batch: Vec<_> = (1..HOST_REPLAY_CHECKPOINT_RECORDS)
        .map(|sequence| {
            event(
                "records",
                sequence as i64,
                &replay_json("records", sequence as i64, 0),
            )
        })
        .collect();
    store.begin_write_unit_of_work().unwrap();
    store.append_host_event_history("records", &batch).unwrap();
    assert!(store
        .checkpoint_replay_ready("records", "records", 2, tiny.len())
        .unwrap());
    store
        .append_host_event_history(
            "records",
            &[event("records", HOST_REPLAY_CHECKPOINT_RECORDS as i64, &tiny)],
        )
        .unwrap();
    assert!(!store
        .checkpoint_replay_ready("records", "records", 2, tiny.len())
        .unwrap());
    store.rollback_write_unit_of_work().unwrap();
    assert!(!store.checkpoint_replay_pending("records").unwrap());
    assert!(store
        .checkpoint_replay_ready("records", "records", 2, tiny.len())
        .unwrap());
    let bytes_overhead = replay_json("bytes", 1, 0).len();
    let exact = replay_json("bytes", 1, HOST_REPLAY_MAX_BYTES - bytes_overhead);
    assert_eq!(exact.len(), HOST_REPLAY_MAX_BYTES);
    assert!(store
        .checkpoint_replay_ready("bytes", "bytes", 2, exact.len())
        .unwrap());
    assert!(!store
        .checkpoint_replay_ready("bytes", "bytes", 2, exact.len() + 1)
        .unwrap());
    store.begin_write_unit_of_work().unwrap();
    store
        .append_host_event_history("bytes", &[event("bytes", 1, &exact)])
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert!(store
        .checkpoint_replay_ready("bytes", "bytes", 2, 0)
        .unwrap());
    assert!(!store
        .checkpoint_replay_ready("bytes", "bytes", 2, 1)
        .unwrap());
    assert_eq!(
        store
            .load_recovery_checkpoint("bytes")
            .unwrap()
            .unwrap()
            .replay_inputs
            .len(),
        1
    );
    baseline(&mut store, "bytes", 2);
    assert!(!store.checkpoint_replay_pending("bytes").unwrap());
    assert!(store
        .load_recovery_checkpoint("bytes")
        .unwrap()
        .unwrap()
        .replay_inputs
        .is_empty());
    assert!(store
        .checkpoint_replay_ready("bytes", "bytes", 3, HOST_REPLAY_MAX_BYTES)
        .unwrap());
    assert!(!store
        .checkpoint_replay_ready("records", "bytes", 2, 1)
        .unwrap());
    let (head,base):(i64,i64)=store.connection.query_row("SELECT h.reduction_bytes,b.through_reduction_bytes
        FROM host_event_replay_heads h JOIN core_checkpoint_replay_bases b USING(run_id) WHERE h.run_id='bytes'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(head, HOST_REPLAY_MAX_BYTES as i64);
    assert_eq!(head, base);
}

#[test]
fn reader_accepts_legal_long_tail_beyond_writer_checkpoint_cadence() {
    let mut store = NolaStore::open(":memory:").unwrap();
    begin(&mut store, "long-reader");
    let batch: Vec<_> = (1..=HOST_REPLAY_MAX_RECORDS).map(|sequence| {
        event("long-reader", sequence as i64,
            &replay_json("long-reader", sequence as i64, 0))
    }).collect();
    store.begin_write_unit_of_work().unwrap();
    store.append_host_event_history("long-reader", &batch).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert!(!store.checkpoint_replay_ready("long-reader", "long-reader", 2, 0).unwrap());
    let recovered = store.load_recovery_checkpoint("long-reader").unwrap().unwrap();
    assert_eq!(recovered.replay_inputs.len(), HOST_REPLAY_MAX_RECORDS);
}
