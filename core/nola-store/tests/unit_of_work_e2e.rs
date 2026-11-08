// SPDX-License-Identifier: Apache-2.0

use nola_store::{
    BeginCommandReceiptInput, BeginCommandReceiptResult, CompleteCommandReceiptOutcome, NolaStore,
};
use tempfile::tempdir;

fn receipt_input(request_id: &str) -> BeginCommandReceiptInput {
    BeginCommandReceiptInput {
        client_id: "unit-of-work-client".to_string(),
        request_id: request_id.to_string(),
        command_type: "list_conversations".to_string(),
        command_json: r#"{"type":"list_conversations","query":{"project_root":null,"non_empty":false,"limit":1024,"offset":0}}"#.to_string(),
        expected_revision: None,
        received_at: "2026-09-04T00:00:00Z".to_string(),
    }
}

#[test]
fn outer_unit_of_work_rolls_back_nested_repository_writes() {
    let directory = tempdir().expect("create store fixture");
    let mut store = NolaStore::open(directory.path().join("nola.db")).expect("open store");

    store
        .begin_write_unit_of_work()
        .expect("begin outer unit of work");
    assert!(matches!(
        store
            .begin_command_receipt(&receipt_input("rolled-back"))
            .expect("write nested receipt"),
        BeginCommandReceiptResult::Started(_)
    ));
    store
        .rollback_write_unit_of_work()
        .expect("rollback outer unit of work");

    assert!(store
        .command_receipt("unit-of-work-client", "rolled-back")
        .expect("load rolled-back receipt")
        .is_none());
}

#[test]
fn outer_unit_of_work_commits_nested_terminal_receipt() {
    let directory = tempdir().expect("create store fixture");
    let mut store = NolaStore::open(directory.path().join("nola.db")).expect("open store");

    store
        .begin_write_unit_of_work()
        .expect("begin outer unit of work");
    store
        .begin_command_receipt(&receipt_input("committed"))
        .expect("write accepted receipt");
    store
        .complete_command_receipt(
            "unit-of-work-client",
            "committed",
            "2026-09-04T00:00:01Z",
            &CompleteCommandReceiptOutcome::Committed {
                committed_revision: Some(7),
                result_json: "{\"ok\":true}".to_string(),
            },
        )
        .expect("write terminal receipt");
    store
        .commit_write_unit_of_work()
        .expect("commit outer unit of work");

    let receipt = store
        .command_receipt("unit-of-work-client", "committed")
        .expect("load committed receipt")
        .expect("receipt exists");
    assert_eq!(receipt.status, "committed");
    assert_eq!(receipt.committed_revision, Some(7));
    assert_eq!(receipt.result_json.as_deref(), Some("{\"ok\":true}"));
}
