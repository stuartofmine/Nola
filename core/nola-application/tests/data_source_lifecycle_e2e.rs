// SPDX-License-Identifier: Apache-2.0
//! Verifies the canonical data-source lifecycle through the public Application boundary.

use nola_application::{
    execute_application_command, ApplicationCommand, ConversationDataSourceOverride, DataSource,
    DataSourceAuthorization, DataSourceAuthorizationState, DataSourceCommand,
    DataSourceConfiguration, DataSourceContentReference, DataSourceContextSelection,
    DataSourceDraft, DataSourceFreshnessPolicy, DataSourceKind, DataSourceLocationKind,
    DataSourceReadResult, DataSourceValidationResult, UpdateDataSourceConfiguration,
};
use nola_domain::data_source::{DataSourceAccessPolicy, DataSourceContract, DataSourceReadMethod};
use nola_store::NolaStore;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn execute<T: DeserializeOwned>(store: &mut NolaStore, command: DataSourceCommand) -> T {
    let value = execute_application_command(store, ApplicationCommand::DataSource { command })
        .expect("application command succeeds");
    serde_json::from_value(value).expect("application result decodes")
}

fn create_consumer(store: &mut NolaStore, id: &str) {
    let result = execute_application_command(
        store,
        ApplicationCommand::CreateConversation {
            input: nola_application::CreateConversation {
                id: id.into(),
                title: "Data source consumer".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        },
    )
    .unwrap();
    assert_eq!(result["created"], true);
}

fn draft() -> DataSourceDraft {
    DataSourceDraft {
        id: "source-1".into(),
        name: "Fixture".into(),
        kind: DataSourceKind::Http,
        configuration: DataSourceConfiguration {
            connector_id: "test.http".into(),
            location_kind: DataSourceLocationKind::HttpUrl,
            location: "https://example.test/data".into(),
            parameters: BTreeMap::from([("maximumBytes".into(), Value::from(1024))]),
            secret_references: Vec::new(),
        },
        access_policy: DataSourceAccessPolicy::DefaultEnabled,
        authorization: DataSourceAuthorization {
            state: DataSourceAuthorizationState::NotRequired,
            capability: None,
            observed_at_ms: 100,
        },
        freshness_policy: DataSourceFreshnessPolicy {
            maximum_age_minutes: 60,
            allow_stale_fallback: false,
        },
        capabilities: vec!["read".into(), "context".into()],
        privacy: "local-first".into(),
        created_at_ms: 100,
        updated_at_ms: 100,
    }
}

fn valid_contract() -> DataSourceContract {
    let mut contract: DataSourceContract = serde_json::from_value(json!({
        "sourceId": "source-1",
        "connectorId": "test.http",
        "connectorVersion": 1,
        "readerDescription": "Bounded test HTTP read",
        "readRecipe": {
            "method": "httpGet",
            "operationId": "test.http.read",
            "configurationKeys": ["maximumBytes"],
            "outputSchema": {"type": "object", "properties": {"title": {"type": "string"}}}
        },
        "schemaFields": ["title"],
        "redactedSample": "one record",
        "recordCount": 1,
        "maximumAgeMinutes": 60,
        "validationState": "verified",
        "validatedAt": "2026-09-04T00:00:00Z",
        "observedAt": "2026-09-04T00:00:00Z",
        "validationDigest": ""
    }))
    .unwrap();
    contract.seal_validation();
    assert_eq!(contract.read_recipe.method, DataSourceReadMethod::HttpGet);
    assert!(contract.read_recipe.is_structurally_usable());
    contract
}

#[test]
fn validation_token_revision_and_enablement_are_enforced() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(temporary.path().join("nola.db")).unwrap();

    let created: DataSource = execute(
        &mut store,
        DataSourceCommand::CreateDraft { input: draft() },
    );
    assert!(!created.default_enabled);

    let rejected = execute_application_command(
        &mut store,
        ApplicationCommand::DataSource {
            command: DataSourceCommand::SetDefaultEnabled {
                id: created.id.clone(),
                expected_revision: created.revision,
                enabled: true,
                updated_at_ms: 101,
            },
        },
    );
    assert!(rejected.is_err());

    let pending: DataSource = execute(
        &mut store,
        DataSourceCommand::RequestValidation {
            id: created.id.clone(),
            expected_revision: created.revision,
            requested_at_ms: 102,
        },
    );
    let token = pending.validation.validation_token.clone().unwrap();
    let mut wrong_connector = valid_contract();
    wrong_connector.connector_id = "another.connector".into();
    wrong_connector.seal_validation();
    let rejected_connector = execute_application_command(
        &mut store,
        ApplicationCommand::DataSource {
            command: DataSourceCommand::RecordValidation {
                id: pending.id.clone(),
                validation_token: token.clone(),
                result: DataSourceValidationResult::Succeeded {
                    contract: wrong_connector,
                },
                recorded_at_ms: 103,
            },
        },
    );
    assert!(rejected_connector
        .unwrap_err()
        .contains("connector does not match"));
    let still_pending: DataSource = execute(
        &mut store,
        DataSourceCommand::Get {
            id: pending.id.clone(),
        },
    );
    assert_eq!(still_pending.revision, pending.revision);
    assert_eq!(
        still_pending.validation.validation_token.as_deref(),
        Some(token.as_str())
    );
    assert!(still_pending.validation.contract.is_none());
    let validated: DataSource = execute(
        &mut store,
        DataSourceCommand::RecordValidation {
            id: pending.id.clone(),
            validation_token: token.clone(),
            result: DataSourceValidationResult::Succeeded {
                contract: valid_contract(),
            },
            recorded_at_ms: 103,
        },
    );

    let replay = execute_application_command(
        &mut store,
        ApplicationCommand::DataSource {
            command: DataSourceCommand::RecordValidation {
                id: validated.id.clone(),
                validation_token: token,
                result: DataSourceValidationResult::Succeeded {
                    contract: valid_contract(),
                },
                recorded_at_ms: 104,
            },
        },
    );
    assert!(replay.is_err());

    let enabled: DataSource = execute(
        &mut store,
        DataSourceCommand::SetDefaultEnabled {
            id: validated.id.clone(),
            expected_revision: validated.revision,
            enabled: true,
            updated_at_ms: 105,
        },
    );
    assert!(enabled.default_enabled);

    let stale_update = execute_application_command(
        &mut store,
        ApplicationCommand::DataSource {
            command: DataSourceCommand::UpdateConfiguration {
                id: enabled.id.clone(),
                expected_revision: created.revision,
                input: UpdateDataSourceConfiguration {
                    name: "Stale overwrite".into(),
                    kind: enabled.kind,
                    configuration: enabled.configuration.clone(),
                    access_policy: enabled.access_policy,
                    authorization: enabled.authorization.clone(),
                    freshness_policy: enabled.freshness_policy.clone(),
                    capabilities: enabled.capabilities.clone(),
                    privacy: enabled.privacy.clone(),
                    updated_at_ms: 106,
                },
            },
        },
    );
    assert!(stale_update.is_err());
}

#[test]
fn conversation_override_is_durable_and_typed() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("nola.db");
    {
        let mut store = NolaStore::open(&path).unwrap();
        create_consumer(&mut store, "conversation-1");
        let _: DataSource = execute(
            &mut store,
            DataSourceCommand::CreateDraft { input: draft() },
        );
        let _: bool = execute(
            &mut store,
            DataSourceCommand::SetConversationOverride {
                conversation_id: "conversation-1".into(),
                source_id: "source-1".into(),
                value: ConversationDataSourceOverride::Enabled,
                updated_at_ms: 200,
            },
        );
    }

    let mut reopened = NolaStore::open(&path).unwrap();
    let value: ConversationDataSourceOverride = execute(
        &mut reopened,
        DataSourceCommand::GetConversationOverride {
            conversation_id: "conversation-1".into(),
            source_id: "source-1".into(),
        },
    );
    assert_eq!(value, ConversationDataSourceOverride::Enabled);
}

#[test]
fn committed_read_is_the_only_context_usable_projection() {
    let temporary = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(temporary.path().join("nola.db")).unwrap();
    create_consumer(&mut store, "conversation-1");
    let created: DataSource = execute(
        &mut store,
        DataSourceCommand::CreateDraft { input: draft() },
    );
    let pending_validation: DataSource = execute(
        &mut store,
        DataSourceCommand::RequestValidation {
            id: created.id.clone(),
            expected_revision: created.revision,
            requested_at_ms: 300,
        },
    );
    let validated: DataSource = execute(
        &mut store,
        DataSourceCommand::RecordValidation {
            id: created.id.clone(),
            validation_token: pending_validation.validation.validation_token.unwrap(),
            result: DataSourceValidationResult::Succeeded {
                contract: valid_contract(),
            },
            recorded_at_ms: 301,
        },
    );
    let enabled: DataSource = execute(
        &mut store,
        DataSourceCommand::SetDefaultEnabled {
            id: validated.id.clone(),
            expected_revision: validated.revision,
            enabled: true,
            updated_at_ms: 302,
        },
    );

    let before_read: Vec<DataSourceContextSelection> = execute(
        &mut store,
        DataSourceCommand::ResolveForConversation {
            conversation_id: "conversation-1".into(),
            now_ms: 1_000,
        },
    );
    assert!(!before_read[0].usable);
    assert!(before_read[0].should_refresh);

    let pending_read: DataSource = execute(
        &mut store,
        DataSourceCommand::RequestRefresh {
            id: enabled.id.clone(),
            expected_revision: enabled.revision,
            reason: "context preparation".into(),
            requested_at_ms: 303,
        },
    );
    let read_token = pending_read.read_state.read_token.clone().unwrap();
    let read: DataSource = execute(
        &mut store,
        DataSourceCommand::RecordRead {
            id: enabled.id,
            read_token: read_token.clone(),
            result: DataSourceReadResult::Succeeded {
                content_reference: DataSourceContentReference {
                    uri: "nola-content://data-source/source-1/read-1".into(),
                    mime_type: "application/json".into(),
                    byte_count: 128,
                    content_version: "read-1".into(),
                },
                observed_at_ms: 1_000,
                schema_version: 1,
                record_count: 1,
            },
            recorded_at_ms: 304,
        },
    );
    assert_eq!(read.last_successful_read_at_ms, Some(304));
    assert_eq!(read.validation.contract, pending_read.validation.contract);

    let selected: Vec<DataSourceContextSelection> = execute(
        &mut store,
        DataSourceCommand::ResolveForConversation {
            conversation_id: "conversation-1".into(),
            now_ms: 2_000,
        },
    );
    assert!(selected[0].usable);
    assert!(selected[0].content_reference.is_some());
    let expired: Vec<DataSourceContextSelection> = execute(
        &mut store,
        DataSourceCommand::ResolveForConversation {
            conversation_id: "conversation-1".into(),
            now_ms: 1_001 + (read.freshness_policy.maximum_age_minutes * 60_000) as i64,
        },
    );
    assert!(!expired[0].usable);
    assert!(expired[0].should_refresh);

    let replay = execute_application_command(
        &mut store,
        ApplicationCommand::DataSource {
            command: DataSourceCommand::RecordRead {
                id: "source-1".into(),
                read_token,
                result: DataSourceReadResult::Empty {
                    observed_at_ms: 2_000,
                    schema_version: 1,
                },
                recorded_at_ms: 305,
            },
        },
    );
    assert!(replay.is_err());
}

#[test]
fn batch_context_selection_is_conversation_scoped_and_durable() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("nola.db");
    let mut store = NolaStore::open(&path).unwrap();
    create_consumer(&mut store, "a");
    create_consumer(&mut store, "b");
    for index in 0..100 {
        let mut input = draft();
        input.id = format!("source-{index}");
        let _: DataSource = execute(&mut store, DataSourceCommand::CreateDraft { input });
        let _: bool = execute(
            &mut store,
            DataSourceCommand::SetConversationOverride {
                conversation_id: "a".into(),
                source_id: format!("source-{index}"),
                value: if index % 2 == 0 {
                    ConversationDataSourceOverride::Enabled
                } else {
                    ConversationDataSourceOverride::Disabled
                },
                updated_at_ms: 200,
            },
        );
    }
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    for conversation_id in ["a", "b"] {
        let selections: Vec<DataSourceContextSelection> = execute(
            &mut store,
            DataSourceCommand::ResolveForConversation {
                conversation_id: conversation_id.into(),
                now_ms: 300,
            },
        );
        assert_eq!(selections.len(), 100);
        for source in selections {
            let index: usize = source
                .source_id
                .strip_prefix("source-")
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(source.enabled, conversation_id == "a" && index % 2 == 0);
            // Enabling a draft must not bypass its validation requirement.
            assert!(!source.usable);
        }
    }
    let _: bool = execute(
        &mut store,
        DataSourceCommand::SetConversationOverride {
            conversation_id: "a".into(),
            source_id: "source-0".into(),
            value: ConversationDataSourceOverride::Inherit,
            updated_at_ms: 400,
        },
    );
    let selected: Vec<DataSourceContextSelection> = execute(
        &mut store,
        DataSourceCommand::ResolveForConversation {
            conversation_id: "a".into(),
            now_ms: 401,
        },
    );
    let inherited = selected
        .iter()
        .find(|source| source.source_id == "source-0")
        .unwrap();
    assert!(!inherited.enabled);
    assert_eq!(
        serde_json::to_value(inherited.reason).unwrap(),
        "disabled_by_default"
    );
}
