// SPDX-License-Identifier: Apache-2.0
//! Auxiliary receipts share the repository transaction/rollback boundary.
use nola_domain::{AuxiliaryInvocationOwner, AuxiliarySourceObservation};
use nola_store::NolaStore;
use nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository;

fn observation() -> AuxiliarySourceObservation {
    serde_json::from_value(serde_json::json!({
        "source_host_id":"source-host", "operation_id":"summary", "attempt_id":"physical",
        "receipt":{"dispatch_permitted":false,
            "operation":{"id":"summary","owner":{"kind":"source_authority","source_host_id":"source-host","authority_id":"epoch"},
                "scenario":"context_summary","configuration":{"provider":"fixture","model":"fixture-model","configuration_revision":1,
                    "configuration_digest":"a".repeat(64),"input_digest":"b".repeat(64),"max_attempts":1,"maximum_duration_ms":1000,
                    "max_output_tokens_per_attempt":100,"total_output_token_budget":100},
                "state":"succeeded","attempts_started":1,"active_attempt_id":null,"cancellation_requested_at_ms":null,"created_at_ms":1,"revision":3},
            "attempt":{"id":"physical","operation_id":"summary","predecessor_id":null,"state":"succeeded",
                "provider":"fixture","model":"fixture-model","usage":{"coverage":"unknown","provider_raw":null,"normalized":null},
                "failure":null,"output":{"items":[]},"created_at_ms":2,"completed_at_ms":3,"revision":2}}
    })).unwrap()
}

fn fixture() -> (tempfile::TempDir, NolaStore, rusqlite::Connection, AuxiliaryInvocationOwner) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auxiliary.sqlite");
    let store = NolaStore::open(&path).unwrap();
    let inspection = rusqlite::Connection::open(&path).unwrap();
    inspection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms)
        VALUES('owner','fixture',1,1,1),('other','other',1,1,1)").unwrap();
    (directory, store, inspection, AuxiliaryInvocationOwner::Conversation { conversation_id:"owner".into() })
}

#[test]
fn source_receipt_and_projection_rollback_together_after_late_failure() {
    let (_directory, mut store, inspection, owner) = fixture();
    let observation = observation();
    inspection.execute_batch("CREATE TRIGGER reject_projection BEFORE UPDATE ON conversations
        WHEN NEW.id='owner' BEGIN SELECT RAISE(ABORT,'injected projection failure'); END").unwrap();
    assert!(store.commit_auxiliary_source_observation(&owner, &observation).is_err());
    assert!(store.auxiliary_source_observation(&owner, "source-host", "summary").unwrap().is_none());
    let revision: i64 = inspection.query_row("SELECT revision FROM conversations WHERE id='owner'", [], |row| row.get(0)).unwrap();
    assert_eq!(revision, 1);
    inspection.execute_batch("DROP TRIGGER reject_projection").unwrap();
    store.commit_auxiliary_source_observation(&owner, &observation).unwrap();
    assert_eq!(store.auxiliary_source_observation(&owner, "source-host", "summary").unwrap(), Some(observation));
    assert!(inspection.is_autocommit());
}

#[test]
fn source_receipt_nested_rejection_preserves_parent_and_outer_rollback() {
    let (_directory, mut store, inspection, owner) = fixture();
    let observation = observation();
    store.begin_write_unit_of_work().unwrap();
    store.commit_auxiliary_source_observation(&owner, &observation).unwrap();
    let foreign = AuxiliaryInvocationOwner::Conversation { conversation_id:"other".into() };
    assert!(store.commit_auxiliary_source_observation(&foreign, &observation).is_err());
    assert_eq!(store.auxiliary_source_observation(&owner, "source-host", "summary").unwrap(), Some(observation.clone()));
    store.rollback_write_unit_of_work().unwrap();
    assert!(store.auxiliary_source_observation(&owner, "source-host", "summary").unwrap().is_none());
    let revision: i64 = inspection.query_row("SELECT revision FROM conversations WHERE id='owner'", [], |row| row.get(0)).unwrap();
    assert_eq!(revision, 1);
    store.commit_auxiliary_source_observation(&owner, &observation).unwrap();
}

#[test]
fn invocation_partial_attempt_failure_and_outer_rollback_leave_no_operation() {
    let (_directory, mut store, _inspection, owner) = fixture();
    let receipt = observation().receipt;
    let mut operation = receipt.operation;
    operation.owner = owner;
    let mut attempt = receipt.attempt.unwrap();
    attempt.operation_id = "foreign-operation".into();
    assert!(store.commit_auxiliary_invocation(&operation, None, Some(&attempt), None).is_err());
    assert!(store.auxiliary_operation("summary").unwrap().is_none());
    assert!(store.auxiliary_attempt("summary", "physical").unwrap().is_none());
    store.begin_write_unit_of_work().unwrap();
    store.commit_auxiliary_invocation(&operation, None, None, None).unwrap();
    assert_eq!(store.auxiliary_operation("summary").unwrap(), Some(operation.clone()));
    store.rollback_write_unit_of_work().unwrap();
    assert!(store.auxiliary_operation("summary").unwrap().is_none());
    store.commit_auxiliary_invocation(&operation, None, None, None).unwrap();
    assert_eq!(store.auxiliary_operation("summary").unwrap(), Some(operation));
}
