// SPDX-License-Identifier: Apache-2.0
//! Real SQLite acceptance coverage for fenced Host-effect delivery attempts.

use nola_domain::{
    EffectDeliveryAttemptMutation, EffectDeliveryAttemptState, EffectDeliveryReplaySafety,
    EffectDeliverySettlement,
};
use nola_kernel::{
    authorize_unknown_effect_retry, claim_effect_delivery_attempt,
    claim_interrupted_effect_delivery_successor, heartbeat_effect_delivery_attempt,
    interrupt_effect_delivery_attempt, mark_effect_delivery_outcome_unknown,
    prepare_effect_delivery_attempt, reclaim_expired_effect_delivery_attempt,
    reject_effect_delivery_before_execution, settle_effect_delivery_attempt,
    start_effect_delivery_execution,
};

#[test]
fn permission_policy_withdrawal_cannot_be_submitted_as_an_unsourced_delivery_mutation() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.begin_turn(&begin_input()).unwrap();
    let claim = initial_claim("policy-attempt","policy-generation","policy-owner","policy-token");
    let claimed = store.claim_effect_delivery_attempts(&[claim]).unwrap().remove(0);
    let withdrawal = nola_kernel::withdraw_unstarted_effect_for_permission_policy(&claimed,120).unwrap();
    assert!(store.commit_effect_delivery_attempt_mutations(&[withdrawal]).unwrap_err().to_string()
        .contains("canonical policy transition"));
    assert_eq!(store.effect_delivery_attempt("policy-attempt").unwrap().unwrap(),claimed);
}

#[test]
fn unknown_effect_retry_rejects_unsourced_manual_assertions_after_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("unknown-effect-retry.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();
    let prepared = prepare_effect_delivery_attempt(
        "unknown-attempt",
        "delivery-run",
        "delivery-effect",
        "delivery-request",
        1,
        EffectDeliveryReplaySafety::RequiresRecoveryDecision,
        None,
        100,
    )
    .unwrap();
    let claimed = claim_effect_delivery_attempt(
        &prepared.record,
        "unknown-generation",
        "unknown-owner",
        "unknown-token",
        200,
        110,
    )
    .unwrap();
    store
        .claim_effect_delivery_attempts(&[EffectDeliveryAttemptMutation {
            expected_revision: None,
            record: claimed.record.clone(),
        }])
        .unwrap();
    let started = start_effect_delivery_execution(
        &claimed.record,
        "unknown-generation",
        "unknown-owner",
        "unknown-token",
        120,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[started.clone()])
        .unwrap();
    let unknown = mark_effect_delivery_outcome_unknown(
        &started.record,
        "unknown-generation",
        "unknown-owner",
        "unknown-token",
        "worker disappeared after execution",
        130,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[unknown.clone()])
        .unwrap();
    let authorization = authorize_unknown_effect_retry(
        &unknown.record,
        "delivery-run",
        "delivery-effect",
        "delivery-request",
        "unknown-attempt",
        "unknown-generation",
        "unknown-owner",
        "unknown-token",
        unknown.record.revision,
        "external audit proves no operation was applied",
        140,
    )
    .unwrap();

    let fault = rusqlite::Connection::open(&database).unwrap();
    fault
        .execute_batch(
            "CREATE TRIGGER reject_unknown_retry_event BEFORE INSERT ON run_events
         WHEN NEW.event_id LIKE 'effect-delivery:unknown-attempt:%'
         BEGIN SELECT RAISE(ABORT,'unknown retry event fault'); END;",
        )
        .unwrap();
    assert!(store
        .authorize_unknown_effect_retry(&authorization)
        .is_err());
    assert_eq!(
        store
            .effect_delivery_attempt("unknown-attempt")
            .unwrap()
            .unwrap()
            .state,
        EffectDeliveryAttemptState::OutcomeUnknown
    );
    fault
        .execute_batch("DROP TRIGGER reject_unknown_retry_event")
        .unwrap();

    let mut first = NolaStore::open(&database).unwrap();
    let mut second = NolaStore::open(&database).unwrap();
    // A free-text assertion is not an original user recovery decision. Only
    // Application's Core-fact transaction may authorize a retry successor.
    assert!(first.authorize_unknown_effect_retry(&authorization).is_err());
    assert!(second
        .authorize_unknown_effect_retry(&authorization)
        .is_err());
    let candidates = second
        .interrupted_effect_delivery_candidates("delivery-run", None, 2, false)
        .unwrap();
    assert!(candidates.is_empty());

    let mut wrong_request = authorization.clone();
    wrong_request.record.request_id = "other-request".into();
    assert!(second
        .authorize_unknown_effect_retry(&wrong_request)
        .is_err());
}
use nola_store::{BeginTurnInput, NolaStore, OutboxEffectInput, STORE_SCHEMA_VERSION};
use nola_store_port::effect_delivery::{InterruptedEffectDeliveryCursor, RuntimeEffectWorkReason};
use nola_store_port::store_maintenance::{RunRecoveryPlan, StoreMaintenanceRepository};
use tempfile::tempdir;

fn begin_input() -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        memory_usage_entries: vec![],
        conversation_id: "delivery-conversation".into(),
        conversation_title: "Effect delivery fencing".into(),
        user_message_id: "delivery-user".into(),
        user_part_id: "delivery-user-part".into(),
        user_text: "perform one external effect".into(),
        user_content_json: None,
        additional_user_parts: Vec::new(),
        assistant_message_id: "delivery-assistant".into(),
        assistant_part_id: "delivery-assistant-part".into(),
        assistant_turn_bindings: vec![],
        run_id: "delivery-run".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 0,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "delivery-config-digest".into(),
        outbox_effects: vec![OutboxEffectInput {
            model_attempt_id: None,
            delegated_invocation_id: None,
            effect_id: "delivery-effect".into(),
            request_id: "delivery-request".into(),
            effect_kind: "tool.execute".into(),
            write: outbox_fixture::tool("delivery-effect", "delivery-request", serde_json::json!({"tool":"write_file","path":"报告.md"})),
        }],
        model_attempt_mutations: Vec::new(),
        delegated_invocation_mutations: Vec::new(),
        delegated_permission_mutations: Vec::new(),
        created_at_ms: 100,
    }
}

fn initial_claim(
    attempt_id: &str,
    generation: &str,
    owner: &str,
    token: &str,
) -> EffectDeliveryAttemptMutation {
    let prepared = prepare_effect_delivery_attempt(
        attempt_id,
        "delivery-run",
        "delivery-effect",
        "delivery-request",
        1,
        EffectDeliveryReplaySafety::Idempotent,
        None,
        100,
    )
    .unwrap();
    let claimed =
        claim_effect_delivery_attempt(&prepared.record, generation, owner, token, 200, 110)
            .unwrap();
    EffectDeliveryAttemptMutation {
        expected_revision: None,
        record: claimed.record,
    }
}

#[test]
fn sqlite_accepts_only_the_kernel_prestart_rejection_settlement() {
    let directory = tempdir().unwrap();
    let database = directory
        .path()
        .join("effect-delivery-prestart-rejection.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();
    let leased = initial_claim("attempt-rejected", "generation", "owner", "token");
    store
        .claim_effect_delivery_attempts(&[leased.clone()])
        .unwrap();

    assert!(settle_effect_delivery_attempt(
        &leased.record,
        "generation",
        "owner",
        "token",
        EffectDeliverySettlement::Failed,
        120,
    )
    .is_err());

    let rejected = reject_effect_delivery_before_execution(
        &leased.record,
        "generation",
        "owner",
        "token",
        "deadline_exceeded: expired before execution",
        120,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[rejected])
        .unwrap();

    let stored = store
        .effect_delivery_attempt("attempt-rejected")
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, EffectDeliveryAttemptState::Settled);
    assert_eq!(
        stored.settlement,
        Some(EffectDeliverySettlement::RejectedBeforeExecution)
    );
    assert!(stored.execution_started_at_ms.is_none());
}

#[test]
fn resumed_effect_claims_the_current_request_and_permanently_fences_its_predecessor() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("effect-delivery-resume-request.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();
    let prepared = prepare_effect_delivery_attempt(
        "attempt-old",
        "delivery-run",
        "delivery-effect",
        "delivery-request",
        1,
        EffectDeliveryReplaySafety::Recoverable,
        None,
        100,
    )
    .unwrap();
    let leased_record = claim_effect_delivery_attempt(
        &prepared.record,
        "generation-old",
        "owner-old",
        "token-old",
        200,
        110,
    )
    .unwrap();
    let leased = EffectDeliveryAttemptMutation {
        expected_revision: None,
        record: leased_record.record,
    };
    store
        .claim_effect_delivery_attempts(&[leased.clone()])
        .unwrap();
    let started = start_effect_delivery_execution(
        &leased.record,
        "generation-old",
        "owner-old",
        "token-old",
        120,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[started.clone()])
        .unwrap();
    let interrupted = interrupt_effect_delivery_attempt(
        &started.record,
        "generation-old",
        "owner-old",
        "token-old",
        nola_domain::EffectDeliveryRecoveryPlan::Resume,
        "transport interrupted",
        130,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[interrupted.clone()])
        .unwrap();
    drop(store);

    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE effect_outbox SET request_id='delivery-request-resumed', binding_json=json_set(binding_json,'$.request_id','delivery-request-resumed') WHERE effect_id='delivery-effect'",
            [],
        )
        .unwrap();
    let mut store = NolaStore::open(&database).unwrap();
    let candidates = store
        .interrupted_effect_delivery_candidates("delivery-run", None, 1, false)
        .unwrap();
    assert_eq!(candidates, vec![interrupted.record.clone()]);

    let successor = claim_interrupted_effect_delivery_successor(
        &candidates[0],
        "attempt-new",
        "delivery-request-resumed",
        "generation-new",
        "owner-new",
        "token-new",
        300,
        140,
    )
    .unwrap();
    let stale_request = claim_interrupted_effect_delivery_successor(
        &candidates[0],
        "attempt-stale-request",
        "delivery-request",
        "generation-stale",
        "owner-stale",
        "token-stale",
        300,
        140,
    )
    .unwrap();
    assert!(store
        .commit_effect_delivery_attempt_mutations(&[stale_request])
        .is_err());
    let mut wrong_run = successor.clone();
    wrong_run.record.id = "attempt-wrong-run".into();
    wrong_run.record.run_id = "run-wrong".into();
    assert!(store
        .commit_effect_delivery_attempt_mutations(&[wrong_run])
        .is_err());
    let mut wrong_effect = successor.clone();
    wrong_effect.record.id = "attempt-wrong-effect".into();
    wrong_effect.record.effect_id = "effect-wrong".into();
    assert!(store
        .commit_effect_delivery_attempt_mutations(&[wrong_effect])
        .is_err());

    store
        .commit_effect_delivery_attempt_mutations(&[successor.clone()])
        .unwrap();
    let stored = store
        .effect_delivery_attempt("attempt-new")
        .unwrap()
        .unwrap();
    assert_eq!(stored.request_id, "delivery-request-resumed");
    assert_eq!(stored.predecessor_id.as_deref(), Some("attempt-old"));
    // Cold recovery must not match the predecessor's old request against the
    // current outbox after rebinding, including repeated startup inspections.
    for _ in 0..2 {
        let mut reopened = NolaStore::open(&database).unwrap();
        let mut observed = false;
        reopened.with_run_recovery_plan(
            Some("delivery-conversation"), "inspect resumed delivery", "after resume", 160,
            |facts| {
                observed = true;
                assert_eq!(facts.effect_delivery_attempts, vec![stored.clone()]);
                Ok(RunRecoveryPlan { preserve_all_traces: true, ..Default::default() })
            },
        ).unwrap();
        assert!(observed);
        assert_eq!(reopened.effect_delivery_attempt("attempt-old").unwrap(), Some(interrupted.record.clone()));
    }

    assert!(start_effect_delivery_execution(
        &stored,
        "generation-old",
        "owner-old",
        "token-old",
        150,
    )
    .is_err());
    assert!(start_effect_delivery_execution(
        &interrupted.record,
        "generation-old",
        "owner-old",
        "token-old",
        150,
    )
    .is_err());
}

#[test]
fn two_sqlite_owners_can_observe_one_candidate_but_only_one_can_claim_it() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("effect-delivery-claim.db");
    let mut first = NolaStore::open(&database).unwrap();
    first.begin_turn(&begin_input()).unwrap();
    let mut second = NolaStore::open(&database).unwrap();

    assert_eq!(
        first
            .pending_effect_delivery_candidates("delivery-run", None, 1)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        second
            .pending_effect_delivery_candidates("delivery-run", None, 1)
            .unwrap()
            .len(),
        1
    );

    let winner = initial_claim("attempt-owner-a", "generation-a", "owner-a", "token-a");
    let loser = initial_claim("attempt-owner-b", "generation-b", "owner-b", "token-b");
    first.claim_effect_delivery_attempts(&[winner]).unwrap();
    let error = second.claim_effect_delivery_attempts(&[loser]).unwrap_err();
    assert!(
        error.to_string().contains("already exists"),
        "unexpected second-owner rejection: {error}"
    );

    let active = second
        .active_effect_delivery_attempt("delivery-effect")
        .unwrap()
        .unwrap();
    assert_eq!(active.id, "attempt-owner-a");
    assert_eq!(active.lease_owner.as_deref(), Some("owner-a"));
    assert_eq!(active.lease_token.as_deref(), Some("token-a"));
    assert!(second
        .pending_effect_delivery_candidates("delivery-run", None, 1)
        .unwrap()
        .is_empty());
}

#[test]
fn two_sqlite_renewals_from_one_revision_have_exactly_one_winner() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("effect-delivery-renew-race.db");
    let mut first = NolaStore::open(&database).unwrap();
    first.begin_turn(&begin_input()).unwrap();
    let leased = initial_claim("renew-attempt", "generation", "owner", "token");
    first
        .claim_effect_delivery_attempts(&[leased.clone()])
        .unwrap();
    let mut second = NolaStore::open(&database).unwrap();
    let first_heartbeat =
        heartbeat_effect_delivery_attempt(&leased.record, "generation", "owner", "token", 250, 120)
            .unwrap();
    let second_heartbeat =
        heartbeat_effect_delivery_attempt(&leased.record, "generation", "owner", "token", 260, 121)
            .unwrap();
    let state_change =
        start_effect_delivery_execution(&leased.record, "generation", "owner", "token", 120)
            .unwrap();
    assert!(first
        .heartbeat_effect_delivery_attempt(&state_change)
        .is_err());

    let renewed = first
        .heartbeat_effect_delivery_attempt(&first_heartbeat)
        .unwrap();
    assert_eq!(renewed.revision, leased.record.revision + 1);
    assert!(second
        .heartbeat_effect_delivery_attempt(&second_heartbeat)
        .is_err());
    assert_eq!(
        second.effect_delivery_attempt("renew-attempt").unwrap(),
        Some(renewed)
    );
}

#[test]
fn heartbeat_event_failure_rolls_back_the_lease_extension() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("effect-delivery-renew-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();
    let leased = initial_claim("renew-rollback", "generation", "owner", "token");
    store
        .claim_effect_delivery_attempts(&[leased.clone()])
        .unwrap();
    let heartbeat =
        heartbeat_effect_delivery_attempt(&leased.record, "generation", "owner", "token", 250, 120)
            .unwrap();
    let fault = rusqlite::Connection::open(&database).unwrap();
    fault
        .execute_batch(
            "CREATE TRIGGER reject_effect_heartbeat_event
             BEFORE INSERT ON run_events
             WHEN NEW.event_kind='effect_delivery_attempt_changed'
             BEGIN SELECT RAISE(ABORT,'fixture heartbeat event failure'); END;",
        )
        .unwrap();

    assert!(store.heartbeat_effect_delivery_attempt(&heartbeat).is_err());
    assert_eq!(
        store.effect_delivery_attempt("renew-rollback").unwrap(),
        Some(leased.record)
    );
}

#[test]
fn heartbeat_loses_cleanly_to_recovery_or_terminal_cas() {
    for terminal_first in [false, true] {
        let directory = tempdir().unwrap();
        let database = directory.path().join(format!(
            "effect-delivery-renew-terminal-{terminal_first}.db"
        ));
        let mut first = NolaStore::open(&database).unwrap();
        first.begin_turn(&begin_input()).unwrap();
        let leased = initial_claim("renew-terminal", "generation", "owner", "token");
        first
            .claim_effect_delivery_attempts(&[leased.clone()])
            .unwrap();
        let started =
            start_effect_delivery_execution(&leased.record, "generation", "owner", "token", 120)
                .unwrap();
        first
            .commit_effect_delivery_attempt_mutations(&[started.clone()])
            .unwrap();
        let heartbeat = heartbeat_effect_delivery_attempt(
            &started.record,
            "generation",
            "owner",
            "token",
            280,
            130,
        )
        .unwrap();
        let terminal = settle_effect_delivery_attempt(
            &started.record,
            "generation",
            "owner",
            "token",
            EffectDeliverySettlement::Succeeded,
            130,
        )
        .unwrap();
        let mut second = NolaStore::open(&database).unwrap();
        if terminal_first {
            second
                .commit_effect_delivery_attempt_mutations(&[terminal.clone()])
                .unwrap();
            assert!(first.heartbeat_effect_delivery_attempt(&heartbeat).is_err());
            assert_eq!(
                first.effect_delivery_attempt("renew-terminal").unwrap(),
                Some(terminal.record)
            );
        } else {
            let renewed = first.heartbeat_effect_delivery_attempt(&heartbeat).unwrap();
            assert!(second
                .commit_effect_delivery_attempt_mutations(&[terminal])
                .is_err());
            assert_eq!(
                second.effect_delivery_attempt("renew-terminal").unwrap(),
                Some(renewed)
            );
        }
    }

    let directory = tempdir().unwrap();
    let database = directory.path().join("effect-delivery-renew-recovery.db");
    let mut renewer = NolaStore::open(&database).unwrap();
    renewer.begin_turn(&begin_input()).unwrap();
    let leased = initial_claim("renew-recovery", "generation", "owner", "token");
    renewer
        .claim_effect_delivery_attempts(&[leased.clone()])
        .unwrap();
    let heartbeat =
        heartbeat_effect_delivery_attempt(&leased.record, "generation", "owner", "token", 280, 130)
            .unwrap();
    let recovery = interrupt_effect_delivery_attempt(
        &leased.record,
        "generation",
        "owner",
        "token",
        nola_domain::EffectDeliveryRecoveryPlan::Retry,
        "worker lease expired before execution started",
        200,
    )
    .unwrap();
    let mut supervisor = NolaStore::open(&database).unwrap();
    supervisor
        .commit_effect_delivery_attempt_mutations(&[recovery.clone()])
        .unwrap();
    assert!(renewer
        .heartbeat_effect_delivery_attempt(&heartbeat)
        .is_err());
    assert_eq!(
        renewer.effect_delivery_attempt("renew-recovery").unwrap(),
        Some(recovery.record)
    );
}

#[test]
fn claim_batch_failure_rolls_back_the_first_insert_and_its_run_event() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("effect-delivery-batch-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();

    let first = initial_claim("attempt-batch-a", "generation-a", "owner-a", "token-a");
    let second = initial_claim("attempt-batch-b", "generation-b", "owner-b", "token-b");
    assert!(store
        .claim_effect_delivery_attempts(&[first, second])
        .is_err());
    assert!(store
        .effect_delivery_attempt("attempt-batch-a")
        .unwrap()
        .is_none());
    assert!(store
        .effect_delivery_attempt("attempt-batch-b")
        .unwrap()
        .is_none());
    assert_eq!(
        store
            .pending_effect_delivery_candidates("delivery-run", None, 1)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn expired_unstarted_lease_is_reclaimed_atomically_and_late_old_token_is_fenced() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("effect-delivery-reclaim.db");
    let mut first = NolaStore::open(&database).unwrap();
    first.begin_turn(&begin_input()).unwrap();
    let initial = initial_claim("attempt-1", "generation-1", "owner-1", "token-1");
    first
        .claim_effect_delivery_attempts(&[initial.clone()])
        .unwrap();

    // A late worker can compute this mutation while its old lease still looks
    // live locally. The durable revision must reject it after another owner
    // atomically reclaims the expired attempt.
    let stale_heartbeat = heartbeat_effect_delivery_attempt(
        &initial.record,
        "generation-1",
        "owner-1",
        "token-1",
        250,
        150,
    )
    .unwrap();

    let reclaim = reclaim_expired_effect_delivery_attempt(
        &initial.record,
        "generation-1",
        "owner-1",
        "token-1",
        "attempt-2",
        "generation-2",
        "owner-2",
        "token-2",
        400,
        200,
    )
    .unwrap();
    first
        .commit_effect_delivery_attempt_mutations(&[
            reclaim.interrupted.clone(),
            reclaim.successor.clone(),
        ])
        .unwrap();

    let mut reopened = NolaStore::open(&database).unwrap();
    let old = reopened
        .effect_delivery_attempt("attempt-1")
        .unwrap()
        .unwrap();
    assert_eq!(old.state, EffectDeliveryAttemptState::Interrupted);
    let successor = reopened
        .active_effect_delivery_attempt("delivery-effect")
        .unwrap()
        .unwrap();
    assert_eq!(successor.id, "attempt-2");
    assert_eq!(successor.predecessor_id.as_deref(), Some("attempt-1"));
    assert_eq!(successor.attempt_number, 2);
    assert_eq!(successor.lease_owner.as_deref(), Some("owner-2"));

    let error = reopened
        .commit_effect_delivery_attempt_mutations(&[stale_heartbeat])
        .unwrap_err();
    assert!(
        error.to_string().contains("revision changed"),
        "unexpected stale-token rejection: {error}"
    );
    assert_eq!(
        reopened
            .active_effect_delivery_attempt("delivery-effect")
            .unwrap()
            .unwrap()
            .id,
        "attempt-2"
    );
}

#[test]
fn cold_recovery_records_an_unsafe_started_effect_as_unknown_and_never_recandidates_it() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("effect-delivery-unknown.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        store.begin_turn(&begin_input()).unwrap();
        let prepared = prepare_effect_delivery_attempt(
            "unsafe-attempt",
            "delivery-run",
            "delivery-effect",
            "delivery-request",
            1,
            EffectDeliveryReplaySafety::RequiresRecoveryDecision,
            None,
            100,
        )
        .unwrap();
        let claimed = claim_effect_delivery_attempt(
            &prepared.record,
            "unsafe-generation",
            "unsafe-owner",
            "unsafe-token",
            300,
            110,
        )
        .unwrap();
        store
            .claim_effect_delivery_attempts(&[EffectDeliveryAttemptMutation {
                expected_revision: None,
                record: claimed.record.clone(),
            }])
            .unwrap();
        let started = start_effect_delivery_execution(
            &claimed.record,
            "unsafe-generation",
            "unsafe-owner",
            "unsafe-token",
            120,
        )
        .unwrap();
        store
            .commit_effect_delivery_attempt_mutations(&[started])
            .unwrap();
    }

    let mut recovered = NolaStore::open(&database).unwrap();
    let changed = recovered
        .with_run_recovery_plan(
            Some("delivery-conversation"),
            "process restarted",
            "after crash",
            200,
            |facts| {
                let attempt = facts
                    .effect_delivery_attempts
                    .iter()
                    .find(|attempt| attempt.id == "unsafe-attempt")
                    .ok_or_else(|| "missing started attempt in recovery snapshot".to_string())?;
                let mutation = mark_effect_delivery_outcome_unknown(
                    attempt,
                    "unsafe-generation",
                    "unsafe-owner",
                    "unsafe-token",
                    "worker disappeared after execution started",
                    200,
                )?;
                Ok(RunRecoveryPlan {
                    effect_delivery_attempt_mutations: vec![mutation],
                    preserve_all_traces: true,
                    ..Default::default()
                })
            },
        )
        .unwrap();
    assert_eq!(changed, 1);
    assert_eq!(
        recovered
            .effect_delivery_attempt("unsafe-attempt")
            .unwrap()
            .unwrap()
            .state,
        EffectDeliveryAttemptState::OutcomeUnknown
    );
    assert!(recovered
        .pending_effect_delivery_candidates("delivery-run", None, 1)
        .unwrap()
        .is_empty());

    let mut observed_unknown = false;
    let changed_again = recovered
        .with_run_recovery_plan(
            Some("delivery-conversation"),
            "process restarted again",
            "after second crash",
            300,
            |facts| {
                let attempt = facts
                    .effect_delivery_attempts
                    .iter()
                    .find(|attempt| attempt.id == "unsafe-attempt")
                    .ok_or_else(|| "missing unknown attempt in recovery snapshot".to_string())?;
                assert_eq!(attempt.state, EffectDeliveryAttemptState::OutcomeUnknown);
                observed_unknown = true;
                Ok(RunRecoveryPlan {
                    preserve_all_traces: true,
                    ..Default::default()
                })
            },
        )
        .unwrap();
    assert!(observed_unknown);
    assert_eq!(changed_again, 0);
    assert!(recovered
        .pending_effect_delivery_candidates("delivery-run", None, 1)
        .unwrap()
        .is_empty());
}

#[test]
fn run_recovery_attempt_event_failure_rolls_back_the_attempt_and_run_terminal_state() {
    let directory = tempdir().unwrap();
    let database = directory
        .path()
        .join("effect-delivery-recovery-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    store.begin_turn(&begin_input()).unwrap();
    let claimed = store
        .claim_effect_delivery_attempts(&[initial_claim(
            "recovery-attempt",
            "recovery-generation",
            "recovery-owner",
            "recovery-token",
        )])
        .unwrap()
        .remove(0);
    let interrupted = interrupt_effect_delivery_attempt(
        &claimed,
        "recovery-generation",
        "recovery-owner",
        "recovery-token",
        nola_domain::EffectDeliveryRecoveryPlan::Retry,
        "lease expired before execution",
        200,
    )
    .unwrap();

    let fault = rusqlite::Connection::open(&database).unwrap();
    fault
        .execute_batch(
            "CREATE TRIGGER reject_delivery_recovery_event
             BEFORE INSERT ON run_events
             WHEN NEW.event_kind='effect_delivery_attempt_changed'
             BEGIN SELECT RAISE(ABORT,'fixture recovery event failure'); END;",
        )
        .unwrap();
    let result = store.with_run_recovery_plan(
        Some("delivery-conversation"),
        "worker lease expired",
        "during recovery",
        200,
        |facts| {
            assert_eq!(facts.effect_delivery_attempts, vec![claimed.clone()]);
            Ok(RunRecoveryPlan {
                effect_delivery_attempt_mutations: vec![interrupted.clone()],
                preserve_all_traces: true,
                ..Default::default()
            })
        },
    );
    assert!(result.is_err());
    assert_eq!(
        store.effect_delivery_attempt("recovery-attempt").unwrap(),
        Some(claimed.clone())
    );

    fault
        .execute_batch("DROP TRIGGER reject_delivery_recovery_event")
        .unwrap();
    let mut run_status = None;
    store
        .with_run_recovery_plan(
            Some("delivery-conversation"),
            "observe rollback",
            "after failed recovery",
            201,
            |facts| {
                run_status = facts.run.as_ref().map(|run| run.status.clone());
                assert_eq!(facts.effect_delivery_attempts, vec![claimed.clone()]);
                Ok(RunRecoveryPlan {
                    preserve_all_traces: true,
                    ..Default::default()
                })
            },
        )
        .unwrap();
    assert_eq!(run_status.as_deref(), Some("queued"));
}

#[test]
fn interrupted_tail_query_returns_only_retryable_or_resumable_pending_attempts_once() {
    let directory = tempdir().unwrap();
    let database = directory
        .path()
        .join("effect-delivery-interrupted-candidates.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut input = begin_input();
    input.outbox_effects = ["retry", "resume", "unknown", "settled", "successor"]
        .into_iter()
        .map(|effect_id| OutboxEffectInput {
            model_attempt_id: None,
            delegated_invocation_id: None,
            effect_id: effect_id.into(),
            request_id: format!("request-{effect_id}"),
            effect_kind: "fixture".into(),
            write: outbox_fixture::tool(effect_id, &format!("request-{effect_id}"), serde_json::json!({})),
        })
        .collect();
    store.begin_turn(&input).unwrap();

    let claim = |attempt_id: &str,
                 effect_id: &str,
                 replay_safety: EffectDeliveryReplaySafety,
                 token: &str| {
        let prepared = prepare_effect_delivery_attempt(
            attempt_id,
            "delivery-run",
            effect_id,
            &format!("request-{effect_id}"),
            1,
            replay_safety,
            None,
            100,
        )
        .unwrap();
        let claimed =
            claim_effect_delivery_attempt(&prepared.record, "generation", "owner", token, 200, 110)
                .unwrap();
        EffectDeliveryAttemptMutation {
            expected_revision: None,
            record: claimed.record,
        }
    };
    let retry = claim(
        "candidate-retry",
        "retry",
        EffectDeliveryReplaySafety::Idempotent,
        "token-retry",
    );
    let resume = claim(
        "candidate-resume",
        "resume",
        EffectDeliveryReplaySafety::Recoverable,
        "token-resume",
    );
    let unknown = claim(
        "candidate-unknown",
        "unknown",
        EffectDeliveryReplaySafety::RequiresRecoveryDecision,
        "token-unknown",
    );
    let settled = claim(
        "candidate-settled",
        "settled",
        EffectDeliveryReplaySafety::Idempotent,
        "token-settled",
    );
    let successor = claim(
        "old-with-successor",
        "successor",
        EffectDeliveryReplaySafety::Idempotent,
        "token-successor",
    );
    store
        .claim_effect_delivery_attempts(&[
            retry.clone(),
            resume.clone(),
            unknown.clone(),
            settled.clone(),
            successor.clone(),
        ])
        .unwrap();

    let retry_interrupted = interrupt_effect_delivery_attempt(
        &retry.record,
        "generation",
        "owner",
        "token-retry",
        nola_domain::EffectDeliveryRecoveryPlan::Retry,
        "retry transport",
        120,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[retry_interrupted])
        .unwrap();

    let resume_started =
        start_effect_delivery_execution(&resume.record, "generation", "owner", "token-resume", 120)
            .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[resume_started.clone()])
        .unwrap();
    let resume_interrupted = interrupt_effect_delivery_attempt(
        &resume_started.record,
        "generation",
        "owner",
        "token-resume",
        nola_domain::EffectDeliveryRecoveryPlan::Resume,
        "resume transport",
        130,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[resume_interrupted])
        .unwrap();

    let unknown_started = start_effect_delivery_execution(
        &unknown.record,
        "generation",
        "owner",
        "token-unknown",
        120,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[unknown_started.clone()])
        .unwrap();
    let unknown_terminal = mark_effect_delivery_outcome_unknown(
        &unknown_started.record,
        "generation",
        "owner",
        "token-unknown",
        "unknown outcome",
        130,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[unknown_terminal])
        .unwrap();

    let settled_started = start_effect_delivery_execution(
        &settled.record,
        "generation",
        "owner",
        "token-settled",
        120,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[settled_started.clone()])
        .unwrap();
    let settled_terminal = settle_effect_delivery_attempt(
        &settled_started.record,
        "generation",
        "owner",
        "token-settled",
        EffectDeliverySettlement::Succeeded,
        130,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[settled_terminal])
        .unwrap();

    let reclaimed = reclaim_expired_effect_delivery_attempt(
        &successor.record,
        "generation",
        "owner",
        "token-successor",
        "new-successor",
        "generation-2",
        "owner-2",
        "token-2",
        400,
        200,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[reclaimed.interrupted, reclaimed.successor])
        .unwrap();

    let before_resume = store
        .interrupted_effect_delivery_candidates("delivery-run", None, 64, false)
        .unwrap();
    assert_eq!(
        before_resume
            .iter()
            .map(|attempt| attempt.id.as_str())
            .collect::<Vec<_>>(),
        vec!["candidate-retry"]
    );
    assert_eq!(
        before_resume[0].recovery_plan,
        Some(nola_domain::EffectDeliveryRecoveryPlan::Retry)
    );

    drop(store);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE effect_outbox SET request_id='request-resume-rebound' WHERE effect_id='resume'",
            [],
        )
        .unwrap();
    let store = NolaStore::open(&database).unwrap();
    let mut cursor = None;
    let mut candidates = Vec::new();
    loop {
        let page = store
            .interrupted_effect_delivery_candidates("delivery-run", cursor.as_ref(), 1, false)
            .unwrap();
        let Some(attempt) = page.into_iter().next() else {
            break;
        };
        cursor = Some(InterruptedEffectDeliveryCursor {
            attempt_id: attempt.id.clone(),
        });
        candidates.push(attempt);
    }
    assert_eq!(
        candidates
            .iter()
            .map(|attempt| attempt.id.as_str())
            .collect::<Vec<_>>(),
        vec!["candidate-resume", "candidate-retry"]
    );
    assert_eq!(
        candidates
            .iter()
            .map(|attempt| attempt.recovery_plan.unwrap())
            .collect::<Vec<_>>(),
        vec![
            nola_domain::EffectDeliveryRecoveryPlan::Resume,
            nola_domain::EffectDeliveryRecoveryPlan::Retry,
        ]
    );
}

#[test]
fn cross_run_runtime_schedule_is_bounded_fair_and_exposes_only_due_leases() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("runtime-effect-work.db");
    let mut store = NolaStore::open(&database).unwrap();

    let begin = |conversation: &str, run: &str, effects: &[(&str, &str)], created_at_ms: i64| {
        let mut input = begin_input();
        input.conversation_id = conversation.into();
        input.user_message_id = format!("{run}-user");
        input.user_part_id = format!("{run}-user-part");
        input.assistant_message_id = format!("{run}-assistant");
        input.assistant_part_id = format!("{run}-assistant-part");
        input.run_id = run.into();
        input.created_at_ms = created_at_ms;
        input.outbox_effects = effects
            .iter()
            .map(|(effect_id, request_id)| OutboxEffectInput {
                model_attempt_id: None,
                delegated_invocation_id: None,
                effect_id: (*effect_id).into(),
                request_id: (*request_id).into(),
                effect_kind: "fixture".into(),
                write: outbox_fixture::tool(effect_id, request_id, serde_json::json!({})),
            })
            .collect();
        input
    };
    store
        .begin_turn(&begin(
            "schedule-pending-conversation",
            "schedule-pending-run",
            &[
                ("pending-first", "pending-request-1"),
                ("pending-second", "pending-request-2"),
            ],
            10,
        ))
        .unwrap();
    store
        .begin_turn(&begin(
            "schedule-retry-conversation",
            "schedule-retry-run",
            &[("retry-effect", "retry-request")],
            20,
        ))
        .unwrap();
    store
        .begin_turn(&begin(
            "schedule-resume-conversation",
            "schedule-resume-run",
            &[("resume-effect", "resume-request-old")],
            30,
        ))
        .unwrap();
    store
        .begin_turn(&begin(
            "schedule-lease-conversation",
            "schedule-lease-run",
            &[("lease-effect", "lease-request")],
            40,
        ))
        .unwrap();
    store
        .begin_turn(&begin(
            "schedule-future-conversation",
            "schedule-future-run",
            &[("future-effect", "future-request")],
            1_000,
        ))
        .unwrap();
    store
        .begin_turn(&begin(
            "schedule-unknown-conversation",
            "schedule-unknown-run",
            &[
                ("unknown-pending", "unknown-pending-request"),
                ("unknown-lease", "unknown-lease-request"),
            ],
            1,
        ))
        .unwrap();

    let claim = |attempt_id: &str,
                 run_id: &str,
                 effect_id: &str,
                 request_id: &str,
                 replay_safety: EffectDeliveryReplaySafety,
                 token: &str,
                 expires_at_ms: i64| {
        let prepared = prepare_effect_delivery_attempt(
            attempt_id,
            run_id,
            effect_id,
            request_id,
            1,
            replay_safety,
            None,
            50,
        )
        .unwrap();
        let claimed = claim_effect_delivery_attempt(
            &prepared.record,
            "schedule-generation",
            "schedule-owner",
            token,
            expires_at_ms,
            60,
        )
        .unwrap();
        EffectDeliveryAttemptMutation {
            expected_revision: None,
            record: claimed.record,
        }
    };
    let retry = claim(
        "retry-attempt",
        "schedule-retry-run",
        "retry-effect",
        "retry-request",
        EffectDeliveryReplaySafety::Idempotent,
        "retry-token",
        150,
    );
    let resume = claim(
        "resume-attempt",
        "schedule-resume-run",
        "resume-effect",
        "resume-request-old",
        EffectDeliveryReplaySafety::Recoverable,
        "resume-token",
        150,
    );
    let leased = claim(
        "lease-attempt",
        "schedule-lease-run",
        "lease-effect",
        "lease-request",
        EffectDeliveryReplaySafety::Idempotent,
        "lease-token",
        150,
    );
    let unknown_lease = claim(
        "unknown-lease-attempt",
        "schedule-unknown-run",
        "unknown-lease",
        "unknown-lease-request",
        EffectDeliveryReplaySafety::Idempotent,
        "unknown-lease-token",
        120,
    );
    store
        .claim_effect_delivery_attempts(&[
            retry.clone(),
            resume.clone(),
            leased.clone(),
            unknown_lease,
        ])
        .unwrap();
    let retry = interrupt_effect_delivery_attempt(
        &retry.record,
        "schedule-generation",
        "schedule-owner",
        "retry-token",
        nola_domain::EffectDeliveryRecoveryPlan::Retry,
        "retry",
        70,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[retry])
        .unwrap();
    let resume_started = start_effect_delivery_execution(
        &resume.record,
        "schedule-generation",
        "schedule-owner",
        "resume-token",
        75,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[resume_started.clone()])
        .unwrap();
    let resume = interrupt_effect_delivery_attempt(
        &resume_started.record,
        "schedule-generation",
        "schedule-owner",
        "resume-token",
        nola_domain::EffectDeliveryRecoveryPlan::Resume,
        "resume",
        80,
    )
    .unwrap();
    store
        .commit_effect_delivery_attempt_mutations(&[resume])
        .unwrap();
    drop(store);
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "UPDATE effect_outbox SET request_id='resume-request-new' WHERE effect_id='resume-effect'",
            [],
        )
        .unwrap();

    let first = NolaStore::open(&database).unwrap();
    let second = NolaStore::open(&database).unwrap();
    let scope = [
        "schedule-pending-conversation",
        "schedule-retry-conversation",
        "schedule-resume-conversation",
        "schedule-lease-conversation",
        "schedule-future-conversation",
    ]
    .map(str::to_string);
    let before_expiry = first.runtime_effect_work_schedule(&scope, 100, 64).unwrap();
    assert_eq!(before_expiry.next_wake_at_ms, Some(150));
    assert_eq!(before_expiry.runs.len(), 4);
    assert_eq!(before_expiry.runs[0].effect_id, "pending-first");
    assert_eq!(before_expiry.runs[1].reason, RuntimeEffectWorkReason::Retry);
    assert_eq!(
        before_expiry.runs[2].reason,
        RuntimeEffectWorkReason::Resume
    );
    let future = &before_expiry.runs[3];
    assert_eq!(future.effect_id, "future-effect");
    assert_eq!(future.ready_at_ms, 100);
    assert_eq!(future.created_at_ms, 1_000);
    assert_eq!(
        before_expiry,
        second
            .runtime_effect_work_schedule(&scope, 100, 64)
            .unwrap()
    );

    let due = first.runtime_effect_work_schedule(&scope, 150, 2).unwrap();
    assert_eq!(due.runs.len(), 2);
    assert_eq!(
        due.runs[0].reason,
        RuntimeEffectWorkReason::RecoverExpiredLease
    );
    assert_eq!(due.runs[0].conversation_id, "schedule-lease-conversation");
    assert_eq!(due.runs[0].attempt_id.as_deref(), Some("lease-attempt"));
    assert_eq!(due.runs[1].effect_id, "pending-first");
    assert_eq!(due.next_wake_at_ms, None);
    assert!(first.runtime_effect_work_schedule(&scope, 0, 0).is_err());
    assert!(first.runtime_effect_work_schedule(&[], 0, 1).is_err());
    assert!(first
        .runtime_effect_work_schedule(&["  ".into()], 0, 1)
        .is_err());
    assert!(first
        .runtime_effect_work_schedule(&["duplicate".into(), "duplicate".into()], 0, 1)
        .is_err());
    assert!(first
        .runtime_effect_work_schedule(&vec!["scope".into(); 65], 0, 1)
        .is_err());

    let indexes: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='effect_outbox_runtime_work'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexes, 1);
    assert_eq!(first.schema_version().unwrap(), STORE_SCHEMA_VERSION);
}
#[path = "support/outbox.rs"]
mod outbox_fixture;
