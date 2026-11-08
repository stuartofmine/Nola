// SPDX-License-Identifier: Apache-2.0
//! Real canonical SQLite ledger; no placeholder conversation or Run.
use nola_application::execute_auxiliary_invocation;
use nola_domain::*;
use nola_store::NolaStore;
use serde_json::json;

#[test]
fn source_observations_are_not_physical_rows_and_reject_owner_replacement() {
    use nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository;
    let directory = tempfile::tempdir().unwrap();
    let mut remote = NolaStore::open(&directory.path().join("remote.sqlite")).unwrap();
    let mut local = NolaStore::open(&directory.path().join("local.sqlite")).unwrap();
    let configuration = admit(&mut remote, "template", 1).operation.configuration;
    let mut admitted = execute_auxiliary_invocation(
        &mut remote,
        AuxiliaryInvocationCommand::Admit {
            operation_id: "source-operation".into(),
            owner: AuxiliaryInvocationOwner::SourceAuthority {
                source_host_id: "source-host".into(),
                authority_id: "authorization-epoch".into(),
            },
            scenario: AuxiliaryInvocationScenario::EventSummary,
            configuration,
            at_ms: 1,
        },
    )
    .unwrap();
    let local_owner = AuxiliaryInvocationOwner::Account {
        account_id: "actual-local-account".into(),
    };
    let observe = |local: &mut NolaStore, mut receipt: AuxiliaryInvocationReceipt| {
        receipt.dispatch_permitted = false;
        execute_auxiliary_invocation(
            local,
            AuxiliaryInvocationCommand::ObserveSource {
                local_owner: local_owner.clone(),
                observation: AuxiliarySourceObservation {
                    source_host_id: "source-host".into(),
                    operation_id: "source-operation".into(),
                    attempt_id: "source-operation:physical".into(),
                    receipt,
                },
            },
        )
    };
    observe(&mut local, admitted.clone()).unwrap(); // durable before dispatch
    assert!(local
        .auxiliary_source_slot(&local_owner, &AuxiliaryInvocationScenario::EventSummary)
        .unwrap()
        .is_some());
    let mut sibling = admitted.clone();
    sibling.operation.id = "different-logical-request".into();
    assert!(
        execute_auxiliary_invocation(
            &mut local,
            AuxiliaryInvocationCommand::ObserveSource {
                local_owner: local_owner.clone(),
                observation: AuxiliarySourceObservation {
                    source_host_id: "source-host".into(),
                    operation_id: "different-logical-request".into(),
                    attempt_id: "different-logical-request:physical".into(),
                    receipt: sibling,
                },
            }
        )
        .is_err(),
        "the unresolved worker slot cannot be bypassed by a new UUID or input"
    );
    admitted = start(
        &mut remote,
        &admitted.operation,
        "source-operation:physical",
    );
    observe(&mut local, admitted.clone()).unwrap(); // unknown Running, no resend
    let running = admitted.clone();
    admitted = finish(
        &mut remote,
        "source-operation",
        admitted.attempt.as_ref().unwrap(),
        ModelAttemptState::Succeeded,
        json!({"coverage":"known","provider_raw":{"prompt_tokens":100,"completion_tokens":7},
            "normalized":{"input_tokens":100,"output_tokens":7,"cached_input_tokens":null,"cache_write_input_tokens":null}}),
    );
    observe(&mut local, admitted.clone()).unwrap();
    observe(&mut local, admitted.clone()).unwrap(); // idempotent observation
    let mut duplicate_owner = admitted.clone();
    duplicate_owner.dispatch_permitted = false;
    assert!(
        execute_auxiliary_invocation(
            &mut local,
            AuxiliaryInvocationCommand::ObserveSource {
                local_owner: AuxiliaryInvocationOwner::Account {
                    account_id: "another-local-owner".into()
                },
                observation: AuxiliarySourceObservation {
                    source_host_id: "source-host".into(),
                    operation_id: "source-operation".into(),
                    attempt_id: "source-operation:physical".into(),
                    receipt: duplicate_owner,
                },
            }
        )
        .is_err(),
        "one source physical operation cannot be attributed to two local owners"
    );
    assert!(local
        .auxiliary_source_slot(&local_owner, &AuxiliaryInvocationScenario::EventSummary)
        .unwrap()
        .is_none());
    assert!(observe(&mut local, running).is_err());
    let snapshot = local
        .auxiliary_source_observation(&local_owner, "source-host", "source-operation")
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot.receipt.attempt.unwrap().usage["normalized"]["output_tokens"],
        7
    );
    let database = rusqlite::Connection::open(directory.path().join("local.sqlite")).unwrap();
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM run_attempts", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM auxiliary_source_observations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let mut foreign = admitted.clone();
    foreign.operation.owner = local_owner.clone();
    assert!(observe(&mut local, foreign).is_err());
    let mut multiple = admitted;
    multiple.operation.configuration.max_attempts = 2;
    assert!(observe(&mut local, multiple).is_err());
}

fn admit(store: &mut NolaStore, id: &str, max_attempts: u32) -> AuxiliaryInvocationReceipt {
    execute_auxiliary_invocation(
        store,
        AuxiliaryInvocationCommand::Admit {
            operation_id: id.into(),
            owner: AuxiliaryInvocationOwner::Account {
                account_id: "account".into(),
            },
            scenario: AuxiliaryInvocationScenario::EventSummary,
            configuration: AuxiliaryInvocationConfiguration {
                provider: "fixture".into(),
                model: Some("original-model".into()),
                configuration_revision: 7,
                configuration_digest: "a".repeat(64),
                input_digest: "b".repeat(64),
                max_attempts,
                maximum_duration_ms: 10000,
                max_output_tokens_per_attempt: Some(16),
                total_output_token_budget: Some(16 * u64::from(max_attempts)),
            },
            at_ms: 1,
        },
    )
    .unwrap()
}
fn start(
    store: &mut NolaStore,
    operation: &AuxiliaryOperation,
    id: &str,
) -> AuxiliaryInvocationReceipt {
    execute_auxiliary_invocation(
        store,
        AuxiliaryInvocationCommand::StartAttempt {
            operation_id: operation.id.clone(),
            attempt_id: id.into(),
            expected_revision: operation.revision,
            at_ms: i64::from(operation.attempts_started) * 2 + 2,
        },
    )
    .unwrap()
}
fn finish(
    store: &mut NolaStore,
    operation: &str,
    attempt: &AuxiliaryModelAttempt,
    state: ModelAttemptState,
    usage: serde_json::Value,
) -> AuxiliaryInvocationReceipt {
    execute_auxiliary_invocation(
        store,
        AuxiliaryInvocationCommand::RecordReceipt {
            operation_id: operation.into(),
            attempt_id: attempt.id.clone(),
            expected_attempt_revision: attempt.revision,
            state,
            usage,
            failure: None,
            output: None,
            at_ms: attempt.created_at_ms + 1,
        },
    )
    .unwrap()
}

#[test]
fn two_failures_and_success_are_exactly_three_physical_attempts_without_any_run() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("canonical.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let mut receipt = admit(&mut store, "operation", 3);
    let unknown = json!({"coverage":"unknown","provider_raw":null,"normalized":null});
    for (id, state) in [
        ("a", ModelAttemptState::Failed),
        ("b", ModelAttemptState::Failed),
        ("c", ModelAttemptState::Succeeded),
    ] {
        let started = start(&mut store, &receipt.operation, id);
        assert!(started.dispatch_permitted);
        let replay = start(&mut store, &receipt.operation, id);
        assert!(!replay.dispatch_permitted);
        receipt = finish(
            &mut store,
            "operation",
            started.attempt.as_ref().unwrap(),
            state,
            unknown.clone(),
        );
    }
    assert_eq!(receipt.operation.attempts_started, 3);
    assert_eq!(receipt.operation.state, AuxiliaryOperationState::Succeeded);
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM runs", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM run_attempts WHERE operation_id='operation' AND run_id IS NULL AND text_part_id IS NULL AND reasoning_part_id IS NULL", [], |row| row.get::<_, i64>(0)).unwrap(), 3);
    let attempt = receipt.attempt.unwrap();
    let known = json!({"coverage":"known","provider_raw":{"prompt_tokens":10,"completion_tokens":4},"normalized":{"input_tokens":10,"output_tokens":4,"cached_input_tokens":null,"cache_write_input_tokens":null}});
    let late = finish(
        &mut store,
        "operation",
        &attempt,
        ModelAttemptState::Succeeded,
        known.clone(),
    );
    let duplicate = finish(
        &mut store,
        "operation",
        &attempt,
        ModelAttemptState::Succeeded,
        known,
    );
    assert_eq!(late.attempt, duplicate.attempt);
    assert_eq!(duplicate.operation.attempts_started, 3);
}

#[test]
fn restart_keeps_running_attempt_and_cancel_does_not_reopen_or_dispatch_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("canonical.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let operation = admit(&mut store, "operation", 1).operation;
    let started = start(&mut store, &operation, "physical");
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert!(!start(&mut store, &operation, "physical").dispatch_permitted);
    let cancelled = execute_auxiliary_invocation(
        &mut store,
        AuxiliaryInvocationCommand::Cancel {
            operation_id: "operation".into(),
            at_ms: 3,
        },
    )
    .unwrap();
    assert!(cancelled.operation.cancellation_requested_at_ms.is_some());
    assert!(execute_auxiliary_invocation(
        &mut store,
        AuxiliaryInvocationCommand::StartAttempt {
            operation_id: "operation".into(),
            attempt_id: "second".into(),
            expected_revision: cancelled.operation.revision,
            at_ms: 4
        }
    )
    .is_err());
    let complete = finish(
        &mut store,
        "operation",
        started.attempt.as_ref().unwrap(),
        ModelAttemptState::Cancelled,
        json!({"coverage":"unknown","provider_raw":null,"normalized":null}),
    );
    assert_eq!(complete.operation.state, AuxiliaryOperationState::Cancelled);
    admit(&mut store, "foreign", 1);
    assert!(execute_auxiliary_invocation(
        &mut store,
        AuxiliaryInvocationCommand::RecordReceipt {
            operation_id: "foreign".into(),
            attempt_id: "physical".into(),
            expected_attempt_revision: 1,
            state: ModelAttemptState::Succeeded,
            usage: json!({"coverage":"unknown","provider_raw":null,"normalized":null}),
            failure: None,
            output: None,
            at_ms: 4
        }
    )
    .is_err());
}

#[test]
fn failed_unknown_usage_keeps_its_total_output_reservation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("canonical.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let mut command = AuxiliaryInvocationCommand::Admit {
        operation_id: "budgeted".into(),
        owner: AuxiliaryInvocationOwner::Account {
            account_id: "account".into(),
        },
        scenario: AuxiliaryInvocationScenario::EventSummary,
        configuration: AuxiliaryInvocationConfiguration {
            provider: "fixture".into(),
            model: None,
            configuration_revision: 1,
            configuration_digest: "a".repeat(64),
            input_digest: "b".repeat(64),
            max_attempts: 3,
            maximum_duration_ms: 10000,
            max_output_tokens_per_attempt: Some(16),
            total_output_token_budget: Some(16),
        },
        at_ms: 1,
    };
    let admitted = execute_auxiliary_invocation(&mut store, command.clone()).unwrap();
    let started = start(&mut store, &admitted.operation, "first");
    let failed = finish(
        &mut store,
        "budgeted",
        started.attempt.as_ref().unwrap(),
        ModelAttemptState::Failed,
        json!({"coverage":"unknown","provider_raw":null,"normalized":null}),
    );
    let error = execute_auxiliary_invocation(
        &mut store,
        AuxiliaryInvocationCommand::StartAttempt {
            operation_id: "budgeted".into(),
            attempt_id: "second".into(),
            expected_revision: failed.operation.revision,
            at_ms: 4,
        },
    )
    .unwrap_err();
    assert!(error.contains("output token budget exhausted"));
    if let AuxiliaryInvocationCommand::Admit { configuration, .. } = &mut command {
        configuration.input_digest = "c".repeat(64);
    }
    assert!(execute_auxiliary_invocation(&mut store, command).is_err());
}
