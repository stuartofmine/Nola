// SPDX-License-Identifier: Apache-2.0
//! Cross-connection visibility of Application-owned model mutations.

use nola_application::{
    execute_application_command, ApplicationCommand, ModelCommand, ModelProfile,
};
use nola_store::NolaStore;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::tempdir;

fn profile(id: &str) -> ModelProfile {
    ModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "fixture".into(),
        model_name: id.into(),
        runtime: "remote".into(),
        source_json: r#"{"baseURL":"https://fixture.invalid/v1"}"#.into(),
        parameters_json: "{}".into(),
        selected: true,
        enabled: true,
        revision: 0,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    }
}

fn exercise_visibility(commit: bool) {
    let directory = tempdir().unwrap();
    let database = directory.path().join("visibility.db");
    {
        let mut seed = NolaStore::open(&database).unwrap();
        execute_application_command(
            &mut seed,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile {
                    input: profile("original"),
                },
            },
        )
        .unwrap();
    }
    // Open before the writer acquires its lock: opening the Store may itself
    // perform maintenance. The observation below is strictly a read operation.
    let reader = NolaStore::open(&database).unwrap();
    let (staged_tx, staged_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let mut store = NolaStore::open(&database).unwrap();
        store.begin_write_unit_of_work().unwrap();
        let expected_revision = store.model_profile("original").unwrap().unwrap().revision;
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::ReplaceProfiles {
                    provider: "fixture".into(),
                    runtime: "remote".into(),
                    profiles: vec![profile("replacement")],
                    updated_at_ms: 20,
                    expected_revisions: std::collections::BTreeMap::from([("original".into(), expected_revision)]),
                },
            },
        )
        .unwrap();
        staged_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        if commit {
            store.commit_write_unit_of_work().unwrap();
        } else {
            store.rollback_write_unit_of_work().unwrap();
        }
    });
    staged_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let during = reader.list_model_profiles().unwrap();
    // Release before assertions so an assertion failure cannot strand a writer.
    release_tx.send(()).unwrap();
    writer.join().unwrap();
    assert_eq!(during.len(), 1);
    assert_eq!(during[0].id, "original");
    assert!(during[0].selected);
    let after = reader.list_model_profiles().unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, if commit { "replacement" } else { "original" });
    assert!(after[0].selected);
}

#[test]
fn replacement_is_not_visible_until_application_unit_of_work_commits() {
    exercise_visibility(true);
}

#[test]
fn application_rollback_keeps_previous_selection_visible_to_other_connection() {
    exercise_visibility(false);
}
