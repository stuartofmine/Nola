// SPDX-License-Identifier: Apache-2.0
use nola_domain::{
    DelegatedInvocationMutation, DelegatedInvocationRecord, DelegatedInvocationState,
};
use nola_store::{BeginTurnInput, CommitTransitionInput, NolaStore, OutboxEffectInput};
use serde_json::json;

fn invocation(
    id: &str,
    state: DelegatedInvocationState,
    revision: i64,
) -> DelegatedInvocationRecord {
    DelegatedInvocationRecord {
        id: id.into(),
        run_id: "run".into(),
        effect_id: "effect".into(),
        text_part_id: format!("{id}:text"),
        reasoning_part_id: format!("{id}:reasoning"),
        state,
        backend_id: "codex".into(),
        backend_session_id: None,
        failure: None,
        usage: json!({}),
        created_at_ms: 1,
        started_at_ms: None,
        completed_at_ms: None,
        revision,
    }
}
fn mutation(
    record: DelegatedInvocationRecord,
    expected_revision: Option<i64>,
) -> DelegatedInvocationMutation {
    DelegatedInvocationMutation {
        expected_revision,
        record,
    }
}
fn effect(attempt: &str) -> OutboxEffectInput {
    OutboxEffectInput {
        delegated_invocation_id: Some(attempt.into()),
        model_attempt_id: None,
        effect_id: "effect".into(),
        request_id: "delivery".into(),
        effect_kind: "delegated_agent".into(),
        write: outbox_fixture::delegated("effect", "delivery", attempt),
    }
}
fn begin(store: &mut NolaStore) {
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            memory_usage_entries: vec![],
            conversation_id: "conversation".into(),
            conversation_title: "Attempts".into(),
            user_message_id: "user".into(),
            user_part_id: "user:part".into(),
            user_text: "fixture".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: "assistant".into(),
            assistant_part_id: "assistant:part".into(),
            assistant_turn_bindings: vec![],
            run_id: "run".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 0,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "digest".into(),
            outbox_effects: vec![effect("first")],
            model_attempt_mutations: vec![],
            delegated_permission_mutations: vec![],
            delegated_invocation_mutations: vec![mutation(
                invocation("first", DelegatedInvocationState::Prepared, 1),
                None,
            )],
            created_at_ms: 1,
        })
        .unwrap();
}
fn commit(
    revision: i64,
    mutations: Vec<DelegatedInvocationMutation>,
    effects: Vec<OutboxEffectInput>,
) -> CommitTransitionInput {
    CommitTransitionInput {
        memory_usage_entries: vec![],
        transition_id: format!("transition-{revision}"),
        run_id: "run".into(),
        expected_checkpoint_revision: revision - 1,
        checkpoint_revision: revision,
        checkpoint_event_cursor: revision,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
        submission_reduction_json: None, host_event: None,
        events: vec![],
        ensured_messages: vec![],
        assistant_turn_bindings: vec![],
        part_message_owners: Default::default(),
        part_mutations: vec![],
        outbox_effects: effects,
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: mutations,
        delegated_permission_mutations: vec![],
        effect_delivery_attempt_mutations: vec![],
        acknowledged_effect_ids: vec![],
        run_status: None,
        assistant_status: None,
        assistant_status_message_id: None,
        committed_at_ms: revision + 1,
    }
}
#[test]
fn session_owner_and_delivery_survive_ack_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delegated.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let mut record = store.delegated_invocation("first").unwrap().unwrap();
    record.state = DelegatedInvocationState::Running;
    record.started_at_ms = Some(2);
    record.backend_session_id = Some("actual-session".into());
    record.revision = 2;
    store
        .commit_transition(&commit(1, vec![mutation(record.clone(), Some(1))], vec![]))
        .unwrap();
    record.state = DelegatedInvocationState::Succeeded;
    record.completed_at_ms = Some(3);
    record.revision = 3;
    let mut finish = commit(2, vec![mutation(record.clone(), Some(2))], vec![]);
    finish.acknowledged_effect_ids = vec!["effect".into()];
    store.commit_transition(&finish).unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert!(store.outbox_effect("effect").unwrap().is_none());
    assert_eq!(
        store.delegated_invocation_for_effect("effect").unwrap(),
        Some(record.clone())
    );
    let owner = store.delegated_request_parts("delivery").unwrap().unwrap();
    assert_eq!(owner.invocation_id, "first");
    assert_eq!(
        store
            .delegated_text_part_owner("conversation", "first:text")
            .unwrap(),
        Some(owner)
    );
    assert!(store.model_request_parts("delivery").unwrap().is_none());
    record.state = DelegatedInvocationState::Running;
    record.revision = 4;
    assert!(store
        .commit_transition(&commit(3, vec![mutation(record, Some(3))], vec![]))
        .is_err());
}
#[test]
fn cas_session_rebinding_and_outbox_failure_roll_back_entire_transition() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let mut record = store.delegated_invocation("first").unwrap().unwrap();
    record.state = DelegatedInvocationState::Running;
    record.backend_session_id = Some("session-one".into());
    record.revision = 2;
    store
        .commit_transition(&commit(1, vec![mutation(record.clone(), Some(1))], vec![]))
        .unwrap();
    for field in ["session", "part", "backend", "run", "effect", "revision"] {
        let mut bad = record.clone();
        bad.revision = 3;
        match field {
            "session" => bad.backend_session_id = Some("session-two".into()),
            "part" => bad.text_part_id = "other-part".into(),
            "backend" => bad.backend_id = "claude".into(),
            "run" => bad.run_id = "other-run".into(),
            "effect" => bad.effect_id = "other-effect".into(),
            _ => bad.revision = 4,
        }
        assert!(
            store
                .commit_transition(&commit(2, vec![mutation(bad, Some(2))], vec![]))
                .is_err(),
            "{field}"
        );
    }
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_delivery BEFORE INSERT ON effect_outbox BEGIN SELECT RAISE(ABORT,'delivery fault'); END").unwrap();
    let mut changed = record.clone();
    changed.state = DelegatedInvocationState::Interrupted;
    changed.revision = 3;
    assert!(store
        .commit_transition(&commit(
            2,
            vec![mutation(changed, Some(2))],
            vec![effect("first")]
        ))
        .is_err());
    assert_eq!(store.delegated_invocation("first").unwrap(), Some(record));
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(1));
}
#[test]
fn independent_identity_unique_effect_and_typed_delivery_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("ownership.db")).unwrap();
    begin(&mut store);
    let second = invocation("second", DelegatedInvocationState::Prepared, 1);
    assert!(store
        .commit_transition(&commit(1, vec![mutation(second, None)], vec![]))
        .is_err());
    let mut delivery = effect("first");
    delivery.model_attempt_id = Some("fake-attempt".into());
    assert!(store
        .commit_transition(&commit(1, vec![], vec![delivery]))
        .is_err());
    let mut delivery = effect("first");
    delivery.effect_id = "other-effect".into();
    assert!(store
        .commit_transition(&commit(1, vec![], vec![delivery]))
        .is_err());
    assert!(store
        .commit_transition(&commit(1, vec![], vec![effect("missing")]))
        .is_err());
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(0));
}
#[test]
fn recovery_reports_orphan_nonterminal_invocation_without_delivery() {
    use nola_store_port::store_maintenance::{RunRecoveryPlan, StoreMaintenanceRepository};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("DELETE FROM effect_outbox", []).unwrap();
    let mut observed = false;
    store
        .with_run_recovery_plan(None, "fixture", "later", 10, |facts| {
            observed = true;
            assert!(facts.effects.is_empty());
            assert_eq!(facts.delegated_invocations.len(), 1);
            assert_eq!(facts.delegated_invocations[0].id, "first");
            Ok(RunRecoveryPlan {
                preserve_all_traces: true,
                ..Default::default()
            })
        })
        .unwrap();
    assert!(observed);
}
#[test]
fn enclosing_command_rollback_keeps_creation_and_mutation_unpublished() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outer-uow.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    begin(&mut store);
    assert!(store.delegated_invocation("first").unwrap().is_some());
    let observer = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        observer
            .query_row("SELECT count(*) FROM delegated_invocations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    store.rollback_write_unit_of_work().unwrap();
    assert!(store.delegated_invocation("first").unwrap().is_none());
    assert!(store.delegated_request_parts("delivery").unwrap().is_none());
    assert!(store.checkpoint_revision("conversation").unwrap().is_none());
    drop(observer);
    begin(&mut store);
    let original = store.delegated_invocation("first").unwrap().unwrap();
    let mut running = original.clone();
    running.state = DelegatedInvocationState::Running;
    running.revision = 2;
    store.begin_write_unit_of_work().unwrap();
    store
        .commit_transition(&commit(1, vec![mutation(running, Some(1))], vec![]))
        .unwrap();
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(store.delegated_invocation("first").unwrap(), Some(original));
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(0));
}

#[test]
fn retained_delivery_identity_cannot_cross_execution_owner_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("typed-delivery.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("INSERT INTO run_attempts(id,run_id,invocation_id,effect_id,text_part_id,reasoning_part_id,predecessor_id,state,revision,record_json) VALUES('model','run','model-invocation','model-effect','model-text','model-reasoning',NULL,'prepared',1,'{}')",[]).unwrap();
    assert!(db
        .execute(
            "INSERT INTO run_attempt_deliveries VALUES('delivery','model')",
            []
        )
        .is_err());
    db.execute(
        "INSERT INTO run_attempt_deliveries VALUES('model-delivery','model')",
        [],
    )
    .unwrap();
    assert!(db
        .execute(
            "INSERT INTO delegated_invocation_deliveries VALUES('model-delivery','first')",
            []
        )
        .is_err());
}

#[test]
fn recovery_retains_terminal_invocation_referenced_by_pending_cancellation() {
    use nola_store_port::store_maintenance::{RunRecoveryPlan, StoreMaintenanceRepository};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("terminal-target.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let mut record = store.delegated_invocation("first").unwrap().unwrap();
    record.state = DelegatedInvocationState::Cancelled;
    record.revision = 2;
    let mut input = commit(1,vec![mutation(record,Some(1))],vec![OutboxEffectInput {
        model_attempt_id:None,delegated_invocation_id:None,effect_id:"cancel-effect".into(),request_id:"cancel-request".into(),effect_kind:"cancel".into(),
        write:outbox_fixture::declare("cancel-effect", "cancel-request", serde_json::json!({"type":"cancel","target_effect_id":"effect","target_not_started":null})),
    }]);
    input.acknowledged_effect_ids = vec!["effect".into()];
    store.commit_transition(&input).unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert!(store.outbox_effect("effect").unwrap().is_none());
    let mut observed = false;
    store
        .with_run_recovery_plan(None, "fixture", "later", 10, |facts| {
            observed = true;
            assert_eq!(facts.effects.len(), 1);
            assert_eq!(facts.effects[0].effect_id, "cancel-effect");
            assert_eq!(facts.delegated_invocations.len(), 1);
            assert_eq!(facts.delegated_invocations[0].id, "first");
            assert_eq!(
                facts.delegated_invocations[0].state,
                DelegatedInvocationState::Cancelled
            );
            Ok(RunRecoveryPlan {
                preserve_all_traces: true,
                ..Default::default()
            })
        })
        .unwrap();
    assert!(observed);
}
#[path = "support/outbox.rs"]
mod outbox_fixture;
