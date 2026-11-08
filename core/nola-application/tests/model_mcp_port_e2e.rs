// SPDX-License-Identifier: Apache-2.0

use nola_application::{
    execute_application_command, ApplicationCommand, McpCommand, McpServer, ModelCommand,
    ModelProfile, ModelRuntime, ModelRuntimePhase,
};
use nola_store::NolaStore;
use tempfile::tempdir;
use std::collections::BTreeMap;

fn profile() -> ModelProfile {
    ModelProfile {
        id: "local:qwen-4b".into(),
        display_name: "Qwen 4B".into(),
        provider: "local".into(),
        model_name: "qwen3.5-4b".into(),
        runtime: "llama.cpp".into(),
        source_json: r#"{"artifact_path":"/models/qwen.gguf"}"#.into(),
        parameters_json: r#"{"temperature":0.7}"#.into(),
        selected: true,
        enabled: true,
        revision: 0,
        created_at_ms: 10,
        updated_at_ms: 10,
        deleted_at_ms: None,
    }
}

#[test]
fn catalog_replacement_cas_rejects_stale_revision_and_membership() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("catalog-cas.db");
    let mut store = NolaStore::open(&database).unwrap();
    execute_application_command(&mut store, ApplicationCommand::Model {
        command: ModelCommand::UpsertProfile { input: profile() },
    }).unwrap();
    let original = store.model_profile("local:qwen-4b").unwrap().unwrap();
    let expected = BTreeMap::from([(original.id.clone(), original.revision)]);

    let mut renamed = profile();
    renamed.display_name = "Updated in second window".into();
    execute_application_command(&mut store, ApplicationCommand::Model {
        command: ModelCommand::ReplaceProfiles {
            provider: "local".into(), runtime: "llama.cpp".into(),
            profiles: vec![renamed.clone()], updated_at_ms: 20,
            expected_revisions: expected.clone(),
        },
    }).unwrap();
    let committed = store.model_profile("local:qwen-4b").unwrap().unwrap();
    assert_eq!(committed.display_name, renamed.display_name);
    assert!(committed.revision > original.revision);

    let stale = execute_application_command(&mut store, ApplicationCommand::Model {
        command: ModelCommand::ReplaceProfiles {
            provider: "local".into(), runtime: "llama.cpp".into(),
            profiles: vec![profile()], updated_at_ms: 30,
            expected_revisions: expected,
        },
    });
    assert!(stale.unwrap_err().contains("revision_conflict"));
    assert_eq!(store.model_profile("local:qwen-4b").unwrap().unwrap(), committed);

    let mut added = profile();
    added.id = "local:new".into();
    added.selected = false;
    execute_application_command(&mut store, ApplicationCommand::Model {
        command: ModelCommand::UpsertProfile { input: added },
    }).unwrap();
    let stale_membership = execute_application_command(&mut store, ApplicationCommand::Model {
        command: ModelCommand::ReplaceProfiles {
            provider: "local".into(), runtime: "llama.cpp".into(),
            profiles: vec![renamed], updated_at_ms: 40,
            expected_revisions: BTreeMap::from([(committed.id.clone(), committed.revision)]),
        },
    });
    assert!(stale_membership.unwrap_err().contains("revision_conflict"));
    assert!(store.model_profile("local:new").unwrap().is_some());
}

#[test]
fn catalog_replacement_command_requires_expected_revisions() {
    let without_snapshot = serde_json::json!({
        "type": "replace_profiles",
        "provider": "local",
        "runtime": "llama.cpp",
        "profiles": [],
        "updated_at_ms": 20
    });
    assert!(serde_json::from_value::<ModelCommand>(without_snapshot).is_err());
    let with_empty_snapshot = serde_json::json!({
        "type": "replace_profiles",
        "provider": "local",
        "runtime": "llama.cpp",
        "profiles": [],
        "updated_at_ms": 20,
        "expected_revisions": {}
    });
    assert!(serde_json::from_value::<ModelCommand>(with_empty_snapshot).is_ok());
}

#[test]
fn model_and_mcp_application_objects_survive_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("application-object-ports.db");

    {
        let mut store = NolaStore::open(&database).unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile { input: profile() },
            },
        )
        .unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::ReportRuntime {
                    input: ModelRuntime {
                        host_id: "mac-local".into(),
                        profile_id: "local:qwen-4b".into(),
                        phase: ModelRuntimePhase::Ready,
                        endpoint_json: Some(r#"{"url":"http://127.0.0.1:8080"}"#.into()),
                        process_id: Some(42),
                        error: None,
                        started_at_ms: Some(11),
                        stopped_at_ms: None,
                        updated_at_ms: 12,
                    },
                },
            },
        )
        .unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::Mcp {
                command: McpCommand::Upsert {
                    input: McpServer {
                        id: "filesystem".into(),
                        display_name: "Filesystem".into(),
                        enabled: true,
                        configuration_json: r#"{"transport":"stdio"}"#.into(),
                        revision: 0,
                        created_at_ms: 13,
                        updated_at_ms: 13,
                        deleted_at_ms: None,
                    },
                },
            },
        )
        .unwrap();
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let profile_value = execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::GetProfile {
                id: "local:qwen-4b".into(),
            },
        },
    )
    .unwrap();
    let restored: Option<ModelProfile> = serde_json::from_value(profile_value).unwrap();
    assert_eq!(restored.unwrap().model_name, "qwen3.5-4b");

    let runtime_value = execute_application_command(
        &mut reopened,
        ApplicationCommand::Model {
            command: ModelCommand::GetRuntime {
                host_id: "mac-local".into(),
            },
        },
    )
    .unwrap();
    let runtime: Option<ModelRuntime> = serde_json::from_value(runtime_value).unwrap();
    assert_eq!(runtime.unwrap().phase, ModelRuntimePhase::Ready);

    let servers_value = execute_application_command(
        &mut reopened,
        ApplicationCommand::Mcp {
            command: McpCommand::List,
        },
    )
    .unwrap();
    let servers: Vec<McpServer> = serde_json::from_value(servers_value).unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].id, "filesystem");
}

#[test]
fn unknown_model_runtime_phase_is_rejected_before_dispatch() {
    let raw = serde_json::json!({
        "type": "model",
        "command": {
            "type": "report_runtime",
            "input": {
                "host_id": "mac-local",
                "profile_id": "local:qwen-4b",
                "phase": "warming_up",
                "endpoint_json": null,
                "process_id": null,
                "error": null,
                "started_at_ms": null,
                "stopped_at_ms": null,
                "updated_at_ms": 12
            }
        }
    });

    assert!(serde_json::from_value::<ApplicationCommand>(raw).is_err());
}

#[test]
fn model_replacement_rolls_back_earlier_writes_when_a_later_profile_is_invalid() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("replacement-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile { input: profile() },
        },
    )
    .unwrap();
    let before = serde_json::to_value(store.list_model_profiles().unwrap()).unwrap();
    let expected_revision = store.model_profile("local:qwen-4b").unwrap().unwrap().revision;
    let mut first = profile();
    first.id = "local:replacement".into();
    let mut invalid = profile();
    invalid.id = "local:invalid".into();
    invalid.selected = false;
    invalid.parameters_json = "not-json".into();
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ReplaceProfiles {
                provider: "local".into(),
                runtime: "llama.cpp".into(),
                profiles: vec![first, invalid],
                updated_at_ms: 20,
                expected_revisions: BTreeMap::from([("local:qwen-4b".into(), expected_revision)]),
            },
        }
    )
    .is_err());
    drop(store);
    let reopened = NolaStore::open(&database).unwrap();
    assert_eq!(
        serde_json::to_value(reopened.list_model_profiles().unwrap()).unwrap(),
        before
    );
}

#[test]
fn model_replacement_and_clear_preserve_other_provider_scopes() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("replacement-scope.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut other = profile();
    other.id = "remote:other".into();
    other.provider = "remote".into();
    other.selected = false;
    for input in [profile(), other] {
        execute_application_command(
            &mut store,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile { input },
            },
        )
        .unwrap();
    }
    let mut replacement = profile();
    replacement.id = "local:new".into();
    let expected_revision = store.model_profile("local:qwen-4b").unwrap().unwrap().revision;
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ReplaceProfiles {
                provider: "local".into(),
                runtime: "llama.cpp".into(),
                profiles: vec![replacement],
                updated_at_ms: 20,
                expected_revisions: BTreeMap::from([("local:qwen-4b".into(), expected_revision)]),
            },
        },
    )
    .unwrap();
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    assert!(store.model_profile("local:qwen-4b").unwrap().is_none());
    assert!(store.model_profile("local:new").unwrap().unwrap().selected);
    assert!(store.model_profile("remote:other").unwrap().is_some());
    let replacement_revision = store.model_profile("local:new").unwrap().unwrap().revision;
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ReplaceProfiles {
                provider: "local".into(),
                runtime: "llama.cpp".into(),
                profiles: vec![],
                updated_at_ms: 30,
                expected_revisions: BTreeMap::from([("local:new".into(), replacement_revision)]),
            },
        },
    )
    .unwrap();
    let profiles = store.list_model_profiles().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, "remote:other");
}

#[test]
fn parameter_reset_is_atomic_and_preserves_model_identity_and_selection() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("parameter-reset.db");
    let mut store = NolaStore::open(&database).unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile { input: profile() },
        },
    )
    .unwrap();
    let before = serde_json::to_value(store.list_model_profiles().unwrap()).unwrap();
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ResetParameters {
                profile_ids: vec!["local:qwen-4b".into(), "missing".into()],
                parameters_json: r#"{"temperature":0.2}"#.into(),
                updated_at_ms: 20,
            },
        }
    )
    .is_err());
    assert_eq!(
        serde_json::to_value(store.list_model_profiles().unwrap()).unwrap(),
        before
    );
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ResetParameters {
                profile_ids: vec!["local:qwen-4b".into()],
                parameters_json: r#"{"temperature":0.2}"#.into(),
                updated_at_ms: 30,
            },
        },
    )
    .unwrap();
    drop(store);
    let reopened = NolaStore::open(&database).unwrap();
    let stored = reopened.model_profile("local:qwen-4b").unwrap().unwrap();
    assert!(stored.selected);
    assert_eq!(stored.source_json, profile().source_json);
    assert_eq!(stored.parameters_json, r#"{"temperature":0.2}"#);
    assert_eq!(stored.updated_at_ms, 30);
}

#[test]
fn catalog_parameter_configuration_preserves_existing_fields_and_rejects_tombstones() {
    let directory = tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("catalog-parameters.db")).unwrap();
    let mut input = profile();
    input.selected = false;
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ConfigureParameters {
                input: input.clone(),
            },
        },
    )
    .unwrap();
    assert!(!store.model_profile(&input.id).unwrap().unwrap().selected);
    let mut current = profile();
    current.source_json = r#"{"artifact_path":"/owned/actual.gguf"}"#.into();
    current.display_name = "Canonical name".into();
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile {
                input: current.clone(),
            },
        },
    )
    .unwrap();
    input.parameters_json = r#"{"temperature":0.1}"#.into();
    execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ConfigureParameters {
                input: input.clone(),
            },
        },
    )
    .unwrap();
    let stored = store.model_profile(&input.id).unwrap().unwrap();
    assert!(stored.selected);
    assert_eq!(stored.source_json, current.source_json);
    assert_eq!(stored.display_name, current.display_name);
    assert_eq!(stored.parameters_json, input.parameters_json);
    store.delete_model_profile(&input.id, 50).unwrap();
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::Model {
            command: ModelCommand::ConfigureParameters {
                input: input.clone()
            },
        }
    )
    .is_err());
    assert!(store.model_profile(&input.id).unwrap().is_none());
}
