// SPDX-License-Identifier: Apache-2.0
//! Sync failures must roll back complete Application model commands.

use nola_application::{
    execute_application_command, ApplicationCommand, ModelCommand, ModelProfile,
};
use nola_store::NolaStore;
use rusqlite::Connection;
use serde_json::Value;
use tempfile::tempdir;
use std::collections::BTreeMap;

fn profile(id: &str, selected: bool) -> ModelProfile {
    ModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "fixture".into(),
        model_name: id.into(),
        runtime: "remote".into(),
        source_json: "{}".into(),
        parameters_json: "{}".into(),
        selected,
        enabled: true,
        revision: 0,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    }
}

fn models(store: &mut NolaStore) -> Value {
    execute_application_command(
        store,
        ApplicationCommand::Model {
            command: ModelCommand::ListProfiles,
        },
    )
    .unwrap()
}

fn sync_rows(connection: &Connection) -> Vec<String> {
    connection
        .prepare(
            "SELECT json_object('sequence',local_sequence,'operation',operation_id,
          'type',entity_type,'id',entity_id,'revision',revision,'deleted',deleted,
          'payload',payload_json,'status',status,'cursor',remote_cursor,
          'created',created_at_ms,'resolved',resolved_at_ms)
         FROM sync_outbox ORDER BY local_sequence",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
}

fn rejects_without_partial_commit(mut command: ModelCommand, failing_id: &str) {
    let directory = tempdir().unwrap();
    let database = directory.path().join("model-sync-failure.db");
    let mut store = NolaStore::open(&database).unwrap();
    for input in [profile("first", true), profile("second", false)] {
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile { input },
            },
        )
        .unwrap();
    }
    if let ModelCommand::ReplaceProfiles { ref mut expected_revisions, .. } = command {
        *expected_revisions = store.list_model_profiles().unwrap().into_iter()
            .filter(|profile| profile.provider == "fixture" && profile.runtime == "remote")
            .map(|profile| (profile.id, profile.revision))
            .collect::<BTreeMap<_, _>>();
    }
    let fixture = Connection::open(&database).unwrap();
    let before_models = models(&mut store);
    let before_sync = sync_rows(&fixture);
    // Fixture-only injection; no production bypass or failure switch.
    fixture
        .execute_batch(&format!(
            "CREATE TRIGGER reject_fixture_sync BEFORE INSERT ON sync_outbox
         WHEN NEW.entity_id='{}'
         BEGIN SELECT RAISE(ABORT, 'fixture sync write rejected'); END;",
            failing_id
        ))
        .unwrap();
    let result = execute_application_command(&mut store, ApplicationCommand::Model { command });
    let error = result.expect_err("sync failure must fail the whole command");
    assert!(
        error.to_string().contains("fixture sync write rejected"),
        "{error}"
    );
    assert_eq!(models(&mut store), before_models);
    assert_eq!(sync_rows(&fixture), before_sync);
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert_eq!(models(&mut reopened), before_models);
    assert_eq!(sync_rows(&fixture), before_sync);
}

#[test]
fn replacement_sync_failure_rolls_back_selection_and_earlier_profiles() {
    rejects_without_partial_commit(
        ModelCommand::ReplaceProfiles {
            provider: "fixture".into(),
            runtime: "remote".into(),
            profiles: vec![profile("new-first", true), profile("new-second", false)],
            updated_at_ms: 20,
            expected_revisions: BTreeMap::new(),
        },
        "new-second",
    );
}

#[test]
fn parameter_reset_sync_failure_rolls_back_all_profiles() {
    rejects_without_partial_commit(
        ModelCommand::ResetParameters {
            profile_ids: vec!["first".into(), "second".into()],
            parameters_json: r#"{"temperature":0.25}"#.into(),
            updated_at_ms: 20,
        },
        "second",
    );
}

#[test]
fn parameter_configuration_sync_failure_preserves_previous_revision() {
    let mut input = profile("second", false);
    input.parameters_json = r#"{"temperature":0.25}"#.into();
    input.updated_at_ms = 20;
    rejects_without_partial_commit(ModelCommand::ConfigureParameters { input }, "second");
}

#[test]
fn replacement_cannot_take_over_deleted_profile_from_another_scope() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("deleted-model-identity.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut foreign = profile("shared-id", false);
    foreign.provider = "another-provider".into();
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile { input: foreign },
        },
    )
    .unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::DeleteProfile {
                id: "shared-id".into(),
                deleted_at_ms: 10,
            },
        },
    )
    .unwrap();
    let fixture = Connection::open(&database).unwrap();
    let before_sync = sync_rows(&fixture);
    let result = execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ReplaceProfiles {
                provider: "fixture".into(),
                runtime: "remote".into(),
                profiles: vec![profile("shared-id", true)],
                updated_at_ms: 20,
                expected_revisions: BTreeMap::new(),
            },
        },
    );
    assert!(
        result.is_err(),
        "A deleted model ID still belongs to its original provider/runtime"
    );
    assert_eq!(models(&mut store), serde_json::json!([]));
    assert_eq!(sync_rows(&fixture), before_sync);
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert_eq!(models(&mut reopened), serde_json::json!([]));
}

#[test]
fn direct_upsert_cannot_change_live_or_deleted_model_scope() {
    for deleted in [false, true] {
        for change_runtime in [false, true] {
            let directory = tempdir().unwrap();
            let database = directory.path().join("upsert-identity.db");
            let mut store = NolaStore::open(&database).unwrap();
            let original = profile("stable-model", true);
            execute_application_command(
                &mut store,
                ApplicationCommand::Model {
                    command: ModelCommand::UpsertProfile {
                        input: original.clone(),
                    },
                },
            )
            .unwrap();
            if deleted {
                execute_application_command(
                    &mut store,
                    ApplicationCommand::Model {
                        command: ModelCommand::DeleteProfile {
                            id: original.id.clone(),
                            deleted_at_ms: 10,
                        },
                    },
                )
                .unwrap();
            }
            let fixture = Connection::open(&database).unwrap();
            let before_models = models(&mut store);
            let before_sync = sync_rows(&fixture);
            let mut changed = original;
            if change_runtime {
                changed.runtime = "other-runtime".into();
            } else {
                changed.provider = "other-provider".into();
            }
            let result = execute_application_command(
                &mut store,
                ApplicationCommand::Model {
                    command: ModelCommand::UpsertProfile { input: changed },
                },
            );
            assert!(
                result.is_err(),
                "scope transfer must fail, deleted={deleted}"
            );
            assert_eq!(models(&mut store), before_models);
            assert_eq!(sync_rows(&fixture), before_sync);
        }
    }
}
