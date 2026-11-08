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
use nola_domain::{
    DelegatedPermissionDecision as Decision, DelegatedPermissionDeliveryMutation as Mutation,
    DelegatedPermissionDeliveryRecord as Permission, DelegatedPermissionDeliveryState as State,
};
fn permission() -> Permission {
    Permission {
        id: "permission".into(),
        run_id: "run".into(),
        invocation_id: "first".into(),
        target_effect_id: "effect".into(),
        worker_generation: "worker-1".into(),
        occurrence_id: "occurrence-1".into(),
        backend_id: "codex".into(),
        backend_session_id: "session".into(),
        rpc_request_id: "rpc-7".into(),
        call_id: "call-9".into(),
        request_json: json!({"command":"echo fixture"}),
        request_digest: "digest".into(),
        control_effect_id: None,
        control_request_id: None,
        decision: None,
        state: State::AwaitingDecision,
        created_at_ms: 2,
        claimed_at_ms: None,
        written_at_ms: None,
        revision: 1,
    }
}
fn setup(store: &mut NolaStore) {
    begin(store);
    let mut owner = store.delegated_invocation("first").unwrap().unwrap();
    owner.backend_session_id = Some("session".into());
    owner.revision = 2;
    store
        .commit_transition(&commit(1, vec![mutation(owner, Some(1))], vec![]))
        .unwrap();
}
fn prepared() -> Permission {
    let mut p = permission();
    p.control_effect_id = Some("control".into());
    p.control_request_id = Some("control-request".into());
    p.decision = Some(Decision::AllowOnce);
    p.state = State::Prepared;
    p.revision = 2;
    p
}
fn insert_and_prepare() -> CommitTransitionInput {
    let mut c = commit(2, vec![], vec![]);
    c.delegated_permission_mutations = vec![
        Mutation {
            expected_revision: None,
            record: permission(),
        },
        Mutation {
            expected_revision: Some(1),
            record: prepared(),
        },
    ];
    c
}
#[test]
fn same_commit_insert_authorize_and_evidence_survives_outbox_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("permission.db");
    let mut store = NolaStore::open(&path).unwrap();
    setup(&mut store);
    let mut c = insert_and_prepare();
    c.acknowledged_effect_ids = vec!["effect".into()];
    store.commit_transition(&c).unwrap();
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert!(store.outbox_effect("effect").unwrap().is_none());
    assert_eq!(
        store.delegated_permission("permission").unwrap(),
        Some(prepared())
    );
    assert_eq!(
        store.delegated_permission_for_control("control").unwrap(),
        Some(prepared())
    );
    assert_eq!(
        store
            .delegated_permission_for_occurrence("first", "occurrence-1")
            .unwrap(),
        Some(prepared())
    );
}
#[test]
fn final_outbox_failure_rolls_back_both_permission_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollback.db");
    let mut store = NolaStore::open(&path).unwrap();
    setup(&mut store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_control BEFORE INSERT ON effect_outbox WHEN NEW.effect_id='control' BEGIN SELECT RAISE(ABORT,'fixture rejection'); END;").unwrap();
    let mut c = insert_and_prepare();
    let mut control = effect("first");
    control.delegated_invocation_id = None;
    control.effect_id = "control".into();
    control.request_id = "control-request".into();
    control.write = outbox_fixture::tool("control", "control-request", serde_json::json!({"operation":"permission-delivery"}));
    c.outbox_effects.push(control);
    assert!(store.commit_transition(&c).is_err());
    assert!(store.delegated_permission("permission").unwrap().is_none());
    db.execute_batch("DROP TRIGGER reject_control;").unwrap();
    store.commit_transition(&c).unwrap();
    assert_eq!(
        store.delegated_permission("permission").unwrap(),
        Some(prepared())
    );
}
#[test]
fn stale_cas_rebinding_and_duplicate_occurrence_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("cas.db")).unwrap();
    setup(&mut store);
    store.commit_transition(&insert_and_prepare()).unwrap();
    for field in [
        "revision",
        "generation",
        "rpc",
        "call",
        "request",
        "digest",
        "decision",
        "control",
        "session",
        "target",
    ] {
        let mut p = prepared();
        p.revision = 3;
        match field {
            "revision" => p.revision = 4,
            "generation" => p.worker_generation = "worker-2".into(),
            "rpc" => p.rpc_request_id = "rpc-other".into(),
            "call" => p.call_id = "call-other".into(),
            "request" => p.request_json = json!({}),
            "digest" => p.request_digest = "other".into(),
            "decision" => p.decision = Some(Decision::Deny),
            "control" => p.control_effect_id = Some("other".into()),
            "session" => p.backend_session_id = "other".into(),
            _ => p.target_effect_id = "other".into(),
        }
        let mut c = commit(3, vec![], vec![]);
        c.delegated_permission_mutations.push(Mutation {
            expected_revision: Some(2),
            record: p,
        });
        assert!(store.commit_transition(&c).is_err(), "{field}");
    }
    let mut duplicate = permission();
    duplicate.id = "duplicate".into();
    let mut c = commit(3, vec![], vec![]);
    c.delegated_permission_mutations.push(Mutation {
        expected_revision: None,
        record: duplicate,
    });
    assert!(store.commit_transition(&c).is_err());
    assert_eq!(
        store.delegated_permission("permission").unwrap(),
        Some(prepared())
    );
}
#[test]
fn recovery_applies_only_planner_evidence_and_rolls_back_planner_failure() {
    use nola_store_port::store_maintenance::{RunRecoveryPlan, StoreMaintenanceRepository};
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("recovery.db")).unwrap();
    setup(&mut store);
    store.commit_transition(&insert_and_prepare()).unwrap();
    let mut claimed = prepared();
    claimed.state = State::Claimed;
    claimed.claimed_at_ms = Some(4);
    claimed.revision = 3;
    let mut c = commit(3, vec![], vec![]);
    c.delegated_permission_mutations.push(Mutation {
        expected_revision: Some(2),
        record: claimed.clone(),
    });
    let mut terminal_owner = store.delegated_invocation("first").unwrap().unwrap();
    terminal_owner.state = DelegatedInvocationState::Cancelled;
    terminal_owner.revision = 3;
    terminal_owner.completed_at_ms = Some(4);
    c.delegated_invocation_mutations
        .push(mutation(terminal_owner.clone(), Some(2)));
    c.acknowledged_effect_ids.push("effect".into());
    store.commit_transition(&c).unwrap();
    drop(store);
    let mut store = NolaStore::open(dir.path().join("recovery.db")).unwrap();
    assert!(store.outbox_effect("effect").unwrap().is_none());
    let failure = store.with_run_recovery_plan(
        Some("conversation"),
        "restart",
        "2026-09-09T00:00:00Z",
        5,
        |facts| {
            assert_eq!(facts.delegated_permissions, vec![claimed.clone()]);
            assert_eq!(facts.delegated_invocations, vec![terminal_owner.clone()]);
            Err("planner rejected unverified evidence".into())
        },
    );
    assert!(failure.is_err());
    assert_eq!(
        store.delegated_permission("permission").unwrap(),
        Some(claimed.clone())
    );
    let mut unknown = claimed.clone();
    unknown.state = State::OutcomeUnknown;
    unknown.revision = 4;
    let db = rusqlite::Connection::open(dir.path().join("recovery.db")).unwrap();
    let before_sequence: i64 = db
        .query_row(
            "SELECT next_event_sequence FROM runs WHERE id='run'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("CREATE TRIGGER reject_permission_recovery BEFORE INSERT ON run_events WHEN NEW.event_kind='delegated_permission_changed' BEGIN SELECT RAISE(ABORT,'event rejected'); END;").unwrap();
    assert!(store
        .with_run_recovery_plan(
            Some("conversation"),
            "restart",
            "2026-09-09T00:00:00Z",
            5,
            |_| Ok(RunRecoveryPlan {
                delegated_permission_mutations: vec![Mutation {
                    expected_revision: Some(3),
                    record: unknown.clone()
                }],
                preserve_all_traces: true,
                ..Default::default()
            })
        )
        .is_err());
    assert_eq!(
        store.delegated_permission("permission").unwrap(),
        Some(claimed.clone())
    );
    assert_eq!(
        db.query_row(
            "SELECT next_event_sequence FROM runs WHERE id='run'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        before_sequence
    );
    db.execute_batch("DROP TRIGGER reject_permission_recovery;")
        .unwrap();

    store
        .with_run_recovery_plan(
            Some("conversation"),
            "restart",
            "2026-09-09T00:00:00Z",
            5,
            |facts| {
                assert_eq!(facts.delegated_permissions, vec![claimed.clone()]);
                assert_eq!(facts.delegated_invocations, vec![terminal_owner.clone()]);
                Ok(RunRecoveryPlan {
                    delegated_permission_mutations: vec![Mutation {
                        expected_revision: Some(3),
                        record: unknown.clone(),
                    }],
                    preserve_all_traces: true,
                    ..Default::default()
                })
            },
        )
        .unwrap();
    assert_eq!(
        store.delegated_permission("permission").unwrap(),
        Some(unknown.clone())
    );
    assert!(store
        .open_delegated_permissions_for_target("effect")
        .unwrap()
        .is_empty());
    let event: String = db
        .query_row(
            "SELECT payload_json FROM run_events WHERE event_kind='delegated_permission_changed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(serde_json::from_str::<Permission>(&event).unwrap(), unknown);
    assert_eq!(
        db.query_row(
            "SELECT next_event_sequence FROM runs WHERE id='run'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        before_sequence + 1
    );
}
#[path = "support/outbox.rs"]
mod outbox_fixture;
