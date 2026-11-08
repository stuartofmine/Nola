// SPDX-License-Identifier: Apache-2.0
use nola_domain::{ModelAttemptMutation, ModelAttemptRecord, ModelAttemptState};
use nola_store::{BeginTurnInput, CommitTransitionInput, NolaStore, OutboxEffectInput};
use serde_json::json;

fn attempt(
    id: &str,
    state: ModelAttemptState,
    revision: i64,
    predecessor: Option<&str>,
) -> ModelAttemptRecord {
    ModelAttemptRecord {
        id: id.into(),
        run_id: "run".into(),
        invocation_id: "invocation".into(),
        effect_id: "effect".into(),
        text_part_id: format!("{id}:text"),
        reasoning_part_id: format!("{id}:reasoning"),
        predecessor_id: predecessor.map(str::to_owned),
        state,
        provider: Some("provider".into()),
        model: Some("model".into()),
        failure: None,
        usage: json!({}),
        created_at_ms: 1,
        started_at_ms: None,
        completed_at_ms: None,
        revision,
    }
}
fn mutation(record: ModelAttemptRecord, expected_revision: Option<i64>) -> ModelAttemptMutation {
    ModelAttemptMutation {
        expected_revision,
        record,
    }
}
fn effect(attempt: &str) -> OutboxEffectInput {
    OutboxEffectInput {
        delegated_invocation_id: None,
        model_attempt_id: Some(attempt.into()),
        effect_id: "effect".into(),
        request_id: "delivery".into(),
        effect_kind: "model_stream".into(),
        write: outbox_fixture::model(
            "effect",
            "delivery",
            &format!("{attempt}:text"),
            &format!("{attempt}:reasoning"),
        ),
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
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![mutation(
                attempt("first", ModelAttemptState::Prepared, 1, None),
                None,
            )],
            created_at_ms: 1,
        })
        .unwrap();
}
fn commit(
    revision: i64,
    mutations: Vec<ModelAttemptMutation>,
    effects: Vec<OutboxEffectInput>,
) -> CommitTransitionInput {
    CommitTransitionInput {
        memory_usage_entries: vec![],
        transition_id: format!("transition-{revision}"),
        run_id: "run".into(),
        expected_checkpoint_revision: revision - 1,
        checkpoint_revision: revision,
        checkpoint_event_cursor: revision,
        checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline {
            checkpoint_json: "{}".into(),
        },
        submission_reduction_json: None, host_event: None,
        events: vec![],
        ensured_messages: vec![],
        assistant_turn_bindings: vec![],
        part_message_owners: Default::default(),
        part_mutations: vec![],
        outbox_effects: effects,
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        effect_delivery_attempt_mutations: vec![],
        model_attempt_mutations: mutations,
        acknowledged_effect_ids: vec![],
        run_status: None,
        assistant_status: None,
        assistant_status_message_id: None,
        committed_at_ms: revision + 1,
    }
}
#[test]
fn attempt_chain_parts_and_delivery_survive_ack_cleanup_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("attempts.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let first = store.model_attempt("first").unwrap().unwrap();
    assert_eq!(first.state, ModelAttemptState::Prepared);
    assert!(store
        .model_attempt_successor("run", "effect", "invocation", "first")
        .unwrap()
        .is_none());
    assert!(store
        .model_attempt_successor("run", "effect", "invocation", "missing")
        .unwrap()
        .is_none());
    let mut running = first.clone();
    running.state = ModelAttemptState::Running;
    running.started_at_ms = Some(2);
    running.revision = 2;
    store
        .commit_transition(&commit(1, vec![mutation(running.clone(), Some(1))], vec![]))
        .unwrap();
    let mut superseded = running;
    superseded.state = ModelAttemptState::Superseded;
    superseded.completed_at_ms = Some(3);
    superseded.revision = 3;
    let second = attempt("second", ModelAttemptState::Running, 1, Some("first"));
    let previous = store.outbox_effect("effect").unwrap().unwrap();
    let request: nola_domain::host_effect::HostRequestEnvelope<
        nola_domain::host_effect::HostRequest,
    > = serde_json::from_str(&previous.payload_json).unwrap();
    let mut binding = nola_store_port::host_replay::CoreHostBindingOverlay::from_request(&request);
    binding.attempt_id = Some("second".into());
    binding.attempt_started = true;
    let mut rebound = effect("second");
    rebound.write = nola_store_port::run_effect::OutboxEffectWrite::Rebind {
        expected_request_id: previous.request_id,
        expected_attempt_id: request.attempt_id,
        binding,
    };
    store
        .commit_transition(&commit(
            2,
            vec![
                mutation(superseded, Some(2)),
                mutation(second.clone(), None),
            ],
            vec![rebound],
        ))
        .unwrap();
    assert_eq!(
        store
            .model_attempt_successor("run", "effect", "invocation", "first")
            .unwrap(),
        Some(second.clone())
    );
    for (run, effect, invocation) in [
        ("foreign-run", "effect", "invocation"),
        ("run", "foreign-effect", "invocation"),
        ("run", "effect", "foreign-invocation"),
    ] {
        assert!(store
            .model_attempt_successor(run, effect, invocation, "first")
            .is_err());
    }
    assert_eq!(
        store
            .model_request_parts("delivery")
            .unwrap()
            .unwrap()
            .attempt_id,
        "second"
    );
    assert_eq!(
        store
            .model_text_part_owner("conversation", "first:text")
            .unwrap()
            .unwrap()
            .attempt_id,
        "first"
    );
    let mut finished = second;
    finished.state = ModelAttemptState::Succeeded;
    finished.revision = 2;
    finished.completed_at_ms = Some(4);
    let mut end = commit(3, vec![mutation(finished.clone(), Some(1))], vec![]);
    end.acknowledged_effect_ids = vec!["effect".into()];
    store.commit_transition(&end).unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(
        store
            .model_attempt_successor("run", "effect", "invocation", "first")
            .unwrap(),
        Some(finished.clone())
    );
    assert!(store.outbox_effect("effect").unwrap().is_none());
    assert_eq!(
        store.active_model_attempt_for_effect("effect").unwrap(),
        Some(finished)
    );
    assert!(store.model_attempt("first").unwrap().is_some());
    assert_eq!(
        store
            .model_request_parts("delivery")
            .unwrap()
            .unwrap()
            .attempt_id,
        "second"
    );
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT active_attempt_id FROM runs", [], |r| r
            .get::<_, Option<String>>(0))
            .unwrap(),
        None
    );
}
#[test]
fn stale_cas_foreign_owner_and_outbox_fault_roll_back_attempt_and_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let initial = store.model_attempt("first").unwrap().unwrap();
    let mut invalid = initial.clone();
    invalid.revision = 2;
    invalid.text_part_id = "changed-owner".into();
    assert!(store
        .commit_transition(&commit(1, vec![mutation(invalid, Some(1))], vec![]))
        .is_err());
    assert_eq!(store.model_attempt("first").unwrap(), Some(initial.clone()));
    let mut stale = initial.clone();
    stale.revision = 3;
    assert!(store
        .commit_transition(&commit(1, vec![mutation(stale, Some(2))], vec![]))
        .is_err());
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_attempt_outbox BEFORE INSERT ON effect_outbox BEGIN SELECT RAISE(ABORT,'attempt outbox fault'); END").unwrap();
    let mut running = initial.clone();
    running.state = ModelAttemptState::Running;
    running.revision = 2;
    assert!(store
        .commit_transition(&commit(
            1,
            vec![mutation(running, Some(1))],
            vec![effect("first")]
        ))
        .is_err());
    assert_eq!(store.model_attempt("first").unwrap(), Some(initial));
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(0));
    assert_eq!(
        db.query_row("SELECT active_attempt_id FROM runs", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "first"
    );
}

#[test]
fn typed_delivery_rejects_missing_foreign_run_and_foreign_effect_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ownership.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let mut missing = effect("missing");
    assert!(store
        .commit_transition(&commit(1, vec![], vec![missing.clone()]))
        .is_err());
    missing.model_attempt_id = Some("first".into());
    missing.effect_id = "foreign-effect".into();
    assert!(store
        .commit_transition(&commit(1, vec![], vec![missing]))
        .is_err());
    let mut foreign = attempt("foreign", ModelAttemptState::Prepared, 1, None);
    foreign.run_id = "other-run".into();
    assert!(store
        .commit_transition(&commit(1, vec![mutation(foreign, None)], vec![]))
        .is_err());
    let mut competing = attempt("competing", ModelAttemptState::Prepared, 1, None);
    competing.effect_id = "other-effect".into();
    assert!(store
        .commit_transition(&commit(1, vec![mutation(competing, None)], vec![]))
        .is_err());
    assert!(store.model_attempt("competing").unwrap().is_none());
    assert_eq!(
        store
            .model_request_parts("delivery")
            .unwrap()
            .unwrap()
            .attempt_id,
        "first"
    );
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(0));
    let db = rusqlite::Connection::open(&path).unwrap();
    let initial_events:i64=db.query_row("SELECT count(*) FROM run_events WHERE event_id='model-attempt:first:1' AND event_kind='model_attempt_changed'",[],|r|r.get(0)).unwrap();
    assert_eq!(initial_events, 1);
    assert_eq!(
        db.query_row("SELECT next_event_sequence FROM runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn recovery_reports_active_attempt_even_when_pending_delivery_is_missing() {
    use nola_store_port::store_maintenance::{RunRecoveryPlan, StoreMaintenanceRepository};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing-delivery.db");
    let mut store = NolaStore::open(&path).unwrap();
    begin(&mut store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("DELETE FROM effect_outbox WHERE effect_id='effect'", [])
        .unwrap();
    let mut observed = false;
    store
        .with_run_recovery_plan(None, "fixture", "after", 10, |facts| {
            observed = true;
            assert_eq!(
                facts.run.as_ref().unwrap().active_attempt_id.as_deref(),
                Some("first")
            );
            assert!(facts.effects.is_empty());
            assert!(facts.model_attempts.is_empty());
            // Storage reports the inconsistency; the Application planner owns refusal.
            Ok(RunRecoveryPlan {
                preserve_all_traces: true,
                ..Default::default()
            })
        })
        .unwrap();
    assert!(observed);
    assert!(store.model_attempt("first").unwrap().is_some());
}
#[path = "support/outbox.rs"]
mod outbox_fixture;
