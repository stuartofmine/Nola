// SPDX-License-Identifier: Apache-2.0
//! Data sources, document sources, MCP editors, product settings, automations, trace port, and workspace ownership.

mod data_source_lifecycle_e2e {
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
        let contract: DataSourceContract = serde_json::from_value(json!({
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
            "observedAt": "2026-09-04T00:00:00Z"
        }))
        .unwrap();
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
}

mod document_source_port_e2e {
    // SPDX-License-Identifier: Apache-2.0

    use nola_application::{
        execute_application_command, ApplicationCommand, DocumentSource, DocumentSourceCommand,
        DocumentSourceSummary,
    };
    use nola_store::NolaStore;
    use serde_json::json;
    use tempfile::tempdir;

    fn source() -> DocumentSource {
        DocumentSource {
            id: "document-1".into(),
            scope: "conversation:1".into(),
            uri: "file:///tmp/brief.md".into(),
            kind: "uploaded_document".into(),
            mime_type: Some("text/markdown".into()),
            sha256: None,
            byte_count: Some(31),
            title: Some("Architecture brief".into()),
            text_content: Some("Application owns protocol objects.".into()),
            metadata: json!({"owner":"user","labels":["architecture","source"]}),
            created_at_ms: 10,
            updated_at_ms: 20,
        }
    }

    #[test]
    fn structured_document_source_survives_cold_reopen_and_search() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("document-source-port.db");
        {
            let mut store = NolaStore::open(&database).unwrap();
            let replaced = execute_application_command(
                &mut store,
                ApplicationCommand::DocumentSource {
                    command: DocumentSourceCommand::Replace {
                        scope: "conversation:1".into(),
                        kind: "uploaded_document".into(),
                        inputs: vec![source()],
                    },
                },
            )
            .unwrap();
            assert_eq!(serde_json::from_value::<usize>(replaced).unwrap(), 1);
        }

        let mut reopened = NolaStore::open(&database).unwrap();
        let listed = execute_application_command(
            &mut reopened,
            ApplicationCommand::DocumentSource {
                command: DocumentSourceCommand::List {
                    scope: "conversation:1".into(),
                    kind: Some("uploaded_document".into()),
                },
            },
        )
        .unwrap();
        let listed: Vec<DocumentSource> = serde_json::from_value(listed).unwrap();
        assert_eq!(listed, vec![source()]);

        let searched = execute_application_command(
            &mut reopened,
            ApplicationCommand::DocumentSource {
                command: DocumentSourceCommand::Search {
                    scope: "conversation:1".into(),
                    query: Some("protocol".into()),
                    kind: None,
                    limit: 10,
                },
            },
        )
        .unwrap();
        let searched: Vec<DocumentSourceSummary> = serde_json::from_value(searched).unwrap();
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0].metadata["labels"][0], "architecture");
    }

    #[test]
    fn document_source_command_rejects_persistence_fields_and_unknown_aliases() {
        let raw = json!({
            "type": "document_source",
            "command": {
                "type": "replace",
                "scope": "conversation:1",
                "kind": "uploaded_document",
                "inputs": [{
                    "id": "document-1",
                    "scope": "conversation:1",
                    "uri": "file:///tmp/brief.md",
                    "kind": "uploaded_document",
                    "mime_type": "text/markdown",
                    "sha256": null,
                    "byte_count": 31,
                    "title": "Architecture brief",
                    "text_content": "Application owns protocol objects.",
                    "metadata": {},
                    "metadata_json": "{}",
                    "created_at_ms": 10,
                    "updated_at_ms": 20
                }]
            }
        });
        assert!(serde_json::from_value::<ApplicationCommand>(raw).is_err());
    }

    #[test]
    fn invalid_replacement_preserves_both_conversation_indexes_after_reopen() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("partitions.db");
        let mut store = NolaStore::open(&path).unwrap();
        let first = source();
        let mut second = source();
        second.id = "document-2".into();
        second.scope = "conversation:2".into();
        for input in [&first, &second] {
            execute_application_command(
                &mut store,
                ApplicationCommand::DocumentSource {
                    command: DocumentSourceCommand::Replace {
                        scope: input.scope.clone(),
                        kind: input.kind.clone(),
                        inputs: vec![input.clone()],
                    },
                },
            )
            .unwrap();
        }
        let mut wrong_kind = first.clone();
        wrong_kind.kind = "other".into();
        for inputs in [
            vec![second.clone()],
            vec![wrong_kind],
            vec![first.clone(), first.clone()],
        ] {
            assert!(execute_application_command(
                &mut store,
                ApplicationCommand::DocumentSource {
                    command: DocumentSourceCommand::Replace {
                        scope: first.scope.clone(),
                        kind: first.kind.clone(),
                        inputs,
                    },
                }
            )
            .is_err());
        }
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        for expected in [first, second] {
            let result = execute_application_command(
                &mut store,
                ApplicationCommand::DocumentSource {
                    command: DocumentSourceCommand::List {
                        scope: expected.scope.clone(),
                        kind: None,
                    },
                },
            )
            .unwrap();
            assert_eq!(
                serde_json::from_value::<Vec<DocumentSource>>(result).unwrap(),
                vec![expected]
            );
        }
    }

    #[test]
    fn document_summary_metadata_budget_counts_encoded_bytes_without_mutating_the_original() {
        let directory = tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("metadata-budget.db")).unwrap();
        let maximum = nola_store_port::document_source::DOCUMENT_SUMMARY_METADATA_MAX_BYTES;
        for (metadata, truncated) in [
            (json!({"x":"x".repeat(maximum - 8)}), false),
            (json!({"x":"x".repeat(maximum - 7)}), true),
            (json!({"x":"\u{1f600}".repeat(1100)}), true),
            (json!({"x":"\n".repeat(2100)}), true),
        ] {
            let mut input = source();
            input.metadata = metadata.clone();
            execute_application_command(
                &mut store,
                ApplicationCommand::DocumentSource {
                    command: DocumentSourceCommand::Replace {
                        scope: input.scope.clone(),
                        kind: input.kind.clone(),
                        inputs: vec![input.clone()],
                    },
                },
            )
            .unwrap();
            let result = execute_application_command(
                &mut store,
                ApplicationCommand::DocumentSource {
                    command: DocumentSourceCommand::Search {
                        scope: input.scope.clone(),
                        query: None,
                        kind: None,
                        limit: 1,
                    },
                },
            )
            .unwrap();
            let summaries: Vec<DocumentSourceSummary> = serde_json::from_value(result).unwrap();
            assert_eq!(summaries.len(), 1);
            assert_eq!(summaries[0].metadata_truncated, truncated);
            assert_eq!(
                summaries[0].metadata,
                if truncated { json!({}) } else { metadata }
            );
            assert!(serde_json::to_vec(&summaries[0].metadata).unwrap().len() <= maximum);
            let original = execute_application_command(
                &mut store,
                ApplicationCommand::DocumentSource {
                    command: DocumentSourceCommand::List {
                        scope: input.scope.clone(),
                        kind: None,
                    },
                },
            )
            .unwrap();
            assert_eq!(
                serde_json::from_value::<Vec<DocumentSource>>(original).unwrap(),
                vec![input]
            );
        }
    }
}

mod mcp_editor_receipt_e2e {
    // SPDX-License-Identifier: Apache-2.0
    //! Real Application receipts and SQLite edit concurrency; no model or UI mocks.

    use crate::support;
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand, McpCommand,
        McpServer,
    };
    use support::ApplicationRuntime;

    fn submit(runtime: &mut ApplicationRuntime, id: &str, command: McpCommand) -> bool {
        let command = ApplicationCommand::Mcp { command };
        let command_json = serde_json::to_string(&command).unwrap();
        let receipt = runtime
            .execute_received_command(
                ExecuteReceivedCommand {
                    client_id: "mcp-editor-test".into(),
                    request_id: id.into(),
                    command_type: "mcp".into(),
                    command_json,
                    expected_revision: None,
                    received_at: "2026-09-06T00:00:00Z".into(),
                },
                command,
            )
            .unwrap();
        matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        )
    }

    fn edit(expected_revision: i64, name: &str) -> McpCommand {
        McpCommand::Configure {
            id: "filesystem".into(),
            expected_revision,
            display_name: name.into(),
            enabled: true,
            configuration_json: r#"{"transport":"stdio"}"#.into(),
            updated_at_ms: 20,
        }
    }

    fn records(runtime: &mut ApplicationRuntime) -> Vec<McpServer> {
        serde_json::from_value(
            runtime
                .execute(ApplicationCommand::Mcp {
                    command: McpCommand::List,
                })
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn batch_failure_rolls_back_earlier_changes_and_rejection_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("mcp-batch.db");
        let create = || McpCommand::Create {
            id: "filesystem".into(),
            display_name: "Files".into(),
            enabled: true,
            configuration_json: r#"{"transport":"stdio"}"#.into(),
            created_at_ms: 10,
        };
        let invalid_batch = || McpCommand::ApplyChanges {
            commands: vec![create(), edit(999, "Invalid revision")],
        };
        let mut runtime = support::open_runtime(&database).unwrap();
        assert!(!submit(&mut runtime, "failed-batch", invalid_batch()));
        assert!(records(&mut runtime).is_empty());
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        assert!(!submit(&mut runtime, "failed-batch", invalid_batch()));
        assert!(records(&mut runtime).is_empty());
        assert!(submit(
            &mut runtime,
            "valid-batch",
            McpCommand::ApplyChanges {
                commands: vec![create(), edit(1, "Configured"),]
            }
        ));
        let saved = records(&mut runtime);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].display_name, "Configured");
        assert!(!submit(
            &mut runtime,
            "unchecked-delete",
            McpCommand::ApplyChanges {
                commands: vec![McpCommand::Delete {
                    id: "filesystem".into(),
                    deleted_at_ms: 30
                },]
            }
        ));
        assert!(!submit(
            &mut runtime,
            "nested-batch",
            McpCommand::ApplyChanges {
                commands: vec![McpCommand::ApplyChanges {
                    commands: vec![create()]
                },]
            }
        ));
        assert_eq!(records(&mut runtime), saved);
    }

    #[test]
    fn create_cannot_replace_live_or_deleted_identity_and_replay_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("mcp-create.db");
        let create = || McpCommand::Create {
            id: "filesystem".into(),
            display_name: "Files".into(),
            enabled: true,
            configuration_json: r#"{"transport":"stdio"}"#.into(),
            created_at_ms: 10,
        };
        let mut runtime = support::open_runtime(&database).unwrap();
        assert!(submit(&mut runtime, "create", create()));
        let original = records(&mut runtime);
        assert!(submit(&mut runtime, "create", create()));
        assert!(!submit(&mut runtime, "duplicate", create()));
        assert_eq!(records(&mut runtime), original);
        assert!(submit(
            &mut runtime,
            "delete",
            McpCommand::DeleteChecked {
                id: "filesystem".into(),
                expected_revision: original[0].revision,
                deleted_at_ms: 20,
            }
        ));
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        assert!(!submit(&mut runtime, "resurrect", create()));
        assert!(submit(&mut runtime, "create", create()));
        assert!(records(&mut runtime).is_empty());
    }

    #[test]
    fn stale_edit_and_delete_cannot_overwrite_or_resurrect_server() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("mcp.db");
        let mut runtime = support::open_runtime(&database).unwrap();
        assert!(submit(
            &mut runtime,
            "create",
            McpCommand::Upsert {
                input: McpServer {
                    id: "filesystem".into(),
                    display_name: "Original".into(),
                    enabled: true,
                    configuration_json: r#"{"transport":"stdio"}"#.into(),
                    revision: 0,
                    created_at_ms: 10,
                    updated_at_ms: 10,
                    deleted_at_ms: None,
                }
            }
        ));
        let original = records(&mut runtime).remove(0);
        assert!(runtime
            .execute(ApplicationCommand::Mcp {
                command: edit(original.revision, "Unreceipted"),
            })
            .is_err());
        assert!(submit(
            &mut runtime,
            "edit",
            edit(original.revision, "New name")
        ));
        let saved = records(&mut runtime).remove(0);
        assert_eq!(saved.created_at_ms, original.created_at_ms);
        assert!(saved.revision > original.revision);
        assert!(!submit(
            &mut runtime,
            "stale-edit",
            edit(original.revision, "Lost update")
        ));
        assert!(!submit(
            &mut runtime,
            "stale-delete",
            McpCommand::DeleteChecked {
                id: saved.id.clone(),
                expected_revision: original.revision,
                deleted_at_ms: 30,
            }
        ));
        assert_eq!(records(&mut runtime), vec![saved.clone()]);
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        assert!(!submit(
            &mut runtime,
            "stale-edit",
            edit(original.revision, "Lost update")
        ));
        assert_eq!(records(&mut runtime), vec![saved.clone()]);
        assert!(submit(
            &mut runtime,
            "delete",
            McpCommand::DeleteChecked {
                id: saved.id,
                expected_revision: saved.revision,
                deleted_at_ms: 30,
            }
        ));
        assert!(!submit(
            &mut runtime,
            "resurrect",
            edit(saved.revision, "Resurrected")
        ));
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        assert!(records(&mut runtime).is_empty());
    }
}

mod product_setting_editor_receipt_e2e {
    // SPDX-License-Identifier: Apache-2.0
    use crate::support;
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
    };
    use serde_json::Value;
    use support::ApplicationRuntime;

    fn command(expected_revision: Option<i64>, value: &str) -> ApplicationCommand {
        ApplicationCommand::PutProductSettingChecked {
            key: "editor.fixture".into(),
            value_json: value.into(),
            expected_revision,
            updated_at_ms: 100,
        }
    }

    fn submit(
        runtime: &mut ApplicationRuntime,
        id: &str,
        command: ApplicationCommand,
    ) -> Option<Value> {
        let command_json = serde_json::to_string(&command).unwrap();
        let receipt = runtime
            .execute_received_command(
                ExecuteReceivedCommand {
                    client_id: "setting-editor".into(),
                    request_id: id.into(),
                    command_type: "put_product_setting_checked".into(),
                    command_json,
                    expected_revision: None,
                    received_at: "2026-09-07T00:00:00Z".into(),
                },
                command,
            )
            .unwrap();
        match receipt.outcome {
            ApplicationCommandReceiptOutcome::Committed { result, .. } => Some(result),
            _ => None,
        }
    }

    fn read(runtime: &mut ApplicationRuntime) -> Value {
        runtime
            .execute(ApplicationCommand::GetProductSetting {
                key: "editor.fixture".into(),
            })
            .unwrap()
    }

    #[test]
    fn concurrent_editors_cannot_overwrite_and_committed_receipt_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("settings.db");
        let mut runtime = support::open_runtime(&db).unwrap();
        let first = submit(&mut runtime, "create", command(None, "[]")).unwrap();
        assert_eq!(first["revision"], 1);
        assert!(submit(&mut runtime, "duplicate-create", command(None, "[1]")).is_none());
        let second = submit(&mut runtime, "edit", command(Some(1), "[2]")).unwrap();
        assert_eq!(second["revision"], 2);
        assert!(submit(&mut runtime, "stale", command(Some(1), "[3]")).is_none());
        assert_eq!(read(&mut runtime), second);
        drop(runtime);
        let mut runtime = support::open_runtime(&db).unwrap();
        assert_eq!(
            submit(&mut runtime, "edit", command(Some(1), "[2]")).unwrap(),
            second
        );
        assert_eq!(read(&mut runtime), second);
    }

    #[test]
    fn invalid_json_or_unadmitted_write_cannot_change_the_setting() {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = support::open_runtime(root.path().join("settings.db")).unwrap();
        assert!(runtime.execute(command(None, "[]")).is_err());
        assert_eq!(read(&mut runtime), Value::Null);
        assert!(submit(&mut runtime, "bad-revision", command(Some(0), "[]")).is_none());
        let first = submit(&mut runtime, "create", command(None, "[]")).unwrap();
        assert!(submit(&mut runtime, "bad-json", command(Some(1), "not JSON")).is_none());
        assert_eq!(read(&mut runtime), first);
    }

    #[test]
    fn rejected_receipt_does_not_turn_into_a_write_when_the_precondition_later_matches() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("settings.db");
        let mut runtime = support::open_runtime(&db).unwrap();
        assert!(submit(&mut runtime, "too-early", command(Some(1), "[99]")).is_none());
        let first = submit(&mut runtime, "create", command(None, "[]")).unwrap();
        drop(runtime);
        let mut runtime = support::open_runtime(&db).unwrap();
        assert!(submit(&mut runtime, "too-early", command(Some(1), "[99]")).is_none());
        assert_eq!(read(&mut runtime), first);
    }

    #[test]
    fn default_chat_preference_validates_schema_and_preserves_receipt_cas_across_restart() {
        let root = tempfile::tempdir().unwrap();
        let db = root.path().join("chat-settings.db");
        let mut runtime = support::open_runtime(&db).unwrap();
        let command = |revision, value: &str| ApplicationCommand::PutProductSettingChecked {
            key: "chat.default_model".into(), value_json: value.into(),
            expected_revision: revision, updated_at_ms: 100,
        };
        let read = |runtime: &mut ApplicationRuntime| runtime.execute(ApplicationCommand::GetProductSetting { key: "chat.default_model".into() }).unwrap();
        assert!(submit(&mut runtime, "malformed", command(None, "{}")).is_none());
        assert_eq!(read(&mut runtime), Value::Null);
        let profile = r#"{"version":1,"selection":{"kind":"profile","profile_id":"api:connection:model"}}"#;
        let saved = submit(&mut runtime, "create-default", command(None, profile)).unwrap();
        let automatic = r#"{"version":1,"selection":{"kind":"automatic"}}"#;
        assert!(submit(&mut runtime, "stale-default", command(None, automatic)).is_none());
        assert_eq!(read(&mut runtime), saved);
        drop(runtime);
        let mut runtime = support::open_runtime(&db).unwrap();
        assert_eq!(submit(&mut runtime, "create-default", command(None, profile)).unwrap(), saved);
        assert_eq!(read(&mut runtime), saved);
        let reset = submit(&mut runtime, "reset-default", command(Some(1), automatic)).unwrap();
        assert_eq!(reset["revision"], 2);
    }
}

mod automation_port_receipt_e2e {
    // SPDX-License-Identifier: Apache-2.0
    use crate::support;
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, AutomationCommand, AutomationDefinition,
        AutomationDefinitionCommand, ExecuteReceivedCommand,
    };
    use nola_store::NolaStore;
    use nola_store_port::automation::AutomationRepository;

    #[test]
    fn conditional_definition_command_preserves_reviewed_revision_and_replay() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("conditional.db");
        let mut store = NolaStore::open(&database).unwrap();
        let stored = store.upsert_automation_definition(&nola_store::StoredAutomationDefinition {
            execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
            id:"review".into(),name:"Script".into(),schedule_format:"script".into(),enabled:false,revision:0,
            metadata_json:"{}".into(),created_at_ms:1,updated_at_ms:1,deleted_at_ms:None,
        }).unwrap();
        let mut runtime = support::open_runtime(&database).unwrap();
        let command = serde_json::to_string(&ApplicationCommand::Automation { command: AutomationCommand::Definition {
            command: AutomationDefinitionCommand::UpdateIfRevision { expected_revision: stored.revision, input: AutomationDefinition {
                execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
                id:stored.id.clone(),name:stored.name.clone(),schedule_format:stored.schedule_format.clone(),enabled:true,
                revision:stored.revision,metadata_json:"{\"approved\":true}".into(),created_at_ms:1,updated_at_ms:2,deleted_at_ms:None,
            } },
        } }).unwrap();
        let envelope = |id: &str| ExecuteReceivedCommand { client_id:"conditional-test".into(),request_id:id.into(),command_type:"automation".into(),command_json:command.clone(),expected_revision:None,received_at:"2026-09-22T00:00:00Z".into() };
        let first = runtime.execute_received_command(envelope("approve"), serde_json::from_str(&command).unwrap()).unwrap();
        assert!(store.automation_definition("review").unwrap().unwrap().enabled);
        let revision = store.automation_definition("review").unwrap().unwrap().revision;
        let outbox = store.pending_sync_changes(100).unwrap().len();
        let replay = runtime.execute_received_command(envelope("approve"), serde_json::from_str(&command).unwrap()).unwrap();
        assert!(replay.replayed); assert_eq!(replay.outcome, first.outcome);
        runtime.execute_received_command(envelope("stale"), serde_json::from_str(&command).unwrap()).unwrap();
        assert_eq!(store.automation_definition("review").unwrap().unwrap().revision, revision);
        assert_eq!(store.pending_sync_changes(100).unwrap().len(), outbox);
    }

    #[test]
    fn automation_port_preserves_receipt_atomicity_and_cold_replay() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("automation-port.db");
        let mut runtime = support::open_runtime(&database).unwrap();
        let inspect = rusqlite::Connection::open(&database).unwrap();
        let command = ApplicationCommand::Automation {
            command: AutomationCommand::Definition {
                command: AutomationDefinitionCommand::Upsert {
                    input: AutomationDefinition {
                        execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
                        id: "daily".into(),
                        name: "Daily review".into(),
                        schedule_format: "0 8 * * *".into(),
                        enabled: true,
                        revision: 1,
                        metadata_json: "{}".into(),
                        created_at_ms: 1,
                        updated_at_ms: 1,
                        deleted_at_ms: None,
                    },
                },
            },
        };
        let json = serde_json::to_string(&command).unwrap();
        let envelope = || ExecuteReceivedCommand {
            client_id: "automation-port-test".into(),
            request_id: "upsert".into(),
            command_type: "automation".into(),
            command_json: json.clone(),
            expected_revision: None,
            received_at: "2026-09-08T00:00:00Z".into(),
        };
        inspect.execute_batch("CREATE TRIGGER fail_automation_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'automation receipt fault'); END;").unwrap();
        let baseline_outbox: i64 = inspect
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        let error = runtime
            .execute_received_command(envelope(), serde_json::from_str(&json).unwrap())
            .unwrap_err();
        assert!(
            error.to_string().contains("automation receipt fault"),
            "{error}"
        );
        let mut store = NolaStore::open(&database).unwrap();
        assert!(
            AutomationRepository::automation_definition(&store.automation_repository(), "daily")
                .unwrap()
                .is_none()
        );
        drop(store);
        assert_eq!(
            inspect
                .query_row("SELECT COUNT(*) FROM command_receipts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            inspect
                .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            baseline_outbox
        );
        inspect
            .execute_batch("DROP TRIGGER fail_automation_receipt")
            .unwrap();
        let committed = runtime
            .execute_received_command(envelope(), serde_json::from_str(&json).unwrap())
            .unwrap();
        assert!(matches!(
            committed.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        let outbox: i64 = inspect
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
            .unwrap();
        drop(runtime);
        let mut runtime = support::open_runtime(&database).unwrap();
        let replay = runtime
            .execute_received_command(envelope(), serde_json::from_str(&json).unwrap())
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, committed.outcome);
        assert_eq!(
            inspect
                .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            outbox
        );
        let mut store = NolaStore::open(&database).unwrap();
        assert_eq!(
            AutomationRepository::automation_definition(&store.automation_repository(), "daily")
                .unwrap()
                .unwrap()
                .name,
            "Daily review"
        );
    }
}

mod trace_port_receipt_e2e {
    // SPDX-License-Identifier: Apache-2.0
    use crate::support;
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
    };
    use nola_agent_engine::agent_trace::{
        AgentTraceEvent, AgentTraceKind, AgentTracePayload, AgentTraceStatus, AgentTraceVisibility,
    };
    use serde_json::{json, Value};
    use support::ApplicationRuntime;

    fn event(sequence: u64) -> AgentTraceEvent {
        AgentTraceEvent {
            schema_version: 1,
            trace_id: "trace".into(),
            run_id: "run".into(),
            conversation_id: "conversation".into(),
            message_id: Some("message".into()),
            turn_id: "turn".into(),
            event_id: format!("event-{sequence}"),
            sequence,
            emitted_at: format!("2026-09-08T00:00:0{sequence}Z"),
            kind: if sequence == 1 {
                AgentTraceKind::RunStarted
            } else {
                AgentTraceKind::RunCompleted
            },
            span_id: Some("span".into()),
            parent_span_id: Some("parent".into()),
            status: if sequence == 1 {
                AgentTraceStatus::Running
            } else {
                AgentTraceStatus::Succeeded
            },
            visibility: AgentTraceVisibility::Summary,
            payload: AgentTracePayload::default(),
        }
    }
    fn command(sequence: u64) -> ApplicationCommand {
        ApplicationCommand::RecordAgentTrace {
            event: event(sequence),
            envelope: Some(json!({"source":"host","sequence":sequence})),
        }
    }
    fn envelope(id: &str, sequence: u64) -> ExecuteReceivedCommand {
        ExecuteReceivedCommand {
            client_id: "trace-port".into(),
            request_id: id.into(),
            command_type: "record_agent_trace".into(),
            command_json: serde_json::to_string(&command(sequence)).unwrap(),
            expected_revision: None,
            received_at: "2026-09-08T00:00:00Z".into(),
        }
    }
    fn events(runtime: &mut ApplicationRuntime, cursor: i64) -> Value {
        runtime
            .execute(ApplicationCommand::TraceEventsAfter {
                trace_id: "trace".into(),
                sequence: cursor,
            })
            .unwrap()
    }
    fn runs(runtime: &mut ApplicationRuntime, path: &std::path::Path) -> Value {
        // Diagnostic trace records alone never establish a canonical assistant Run.
        assert_eq!(runtime.execute(ApplicationCommand::TraceRunsForConversation {
            conversation_id: "conversation".into(),
        }).unwrap(), json!([]));
        let db = rusqlite::Connection::open(path).unwrap();
        let mut query = db.prepare("SELECT trace_id,run_id,message_id,turn_id,conversation_id,last_sequence,status,started_at,finished_at FROM trace_journal_runs ORDER BY trace_id").unwrap();
        let rows = query.query_map([], |row| Ok(json!({
            "trace_id": row.get::<_, String>(0)?, "run_id": row.get::<_, String>(1)?,
            "message_id": row.get::<_, Option<String>>(2)?, "turn_id": row.get::<_, String>(3)?,
            "conversation_id": row.get::<_, String>(4)?, "last_sequence": row.get::<_, i64>(5)?,
            "status": row.get::<_, String>(6)?, "started_at": row.get::<_, String>(7)?,
            "finished_at": row.get::<_, Option<String>>(8)?,
        }))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        json!(rows)
    }
    #[test]
    fn trace_port_receipt_failure_preserves_event_run_cursor_and_cold_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trace.db");
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime
            .execute_received_command(envelope("start", 1), command(1))
            .unwrap();
        let baseline_events = events(&mut runtime, 0);
        let baseline_runs = runs(&mut runtime, &path);
        assert_eq!(
            baseline_events,
            json!([{"event":event(1),"envelope":{"source":"host","sequence":1}}])
        );
        assert_eq!(baseline_runs[0]["trace_id"], "trace");
        assert_eq!(baseline_runs[0]["run_id"], "run");
        assert_eq!(baseline_runs[0]["message_id"], "message");
        assert_eq!(baseline_runs[0]["turn_id"], "turn");
        assert_eq!(baseline_runs[0]["conversation_id"], "conversation");
        assert_eq!(baseline_runs[0]["last_sequence"], 1);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER fail_trace_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'trace receipt fault'); END").unwrap();
        assert!(runtime
            .execute_received_command(envelope("failed", 2), command(2))
            .is_err());
        assert_eq!(events(&mut runtime, 0), baseline_events);
        assert_eq!(runs(&mut runtime, &path), baseline_runs);
        assert_eq!(events(&mut runtime, 1), json!([]));
        db.execute_batch("DROP TRIGGER fail_trace_receipt").unwrap();
        let mut receipt = runtime
            .execute_received_command(envelope("finish", 2), command(2))
            .unwrap();
        assert!(matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        assert!(!receipt.replayed);
        receipt.replayed = true;
        let completed_events = events(&mut runtime, 0);
        let completed_runs = runs(&mut runtime, &path);
        assert_eq!(
            events(&mut runtime, 1),
            json!([{"event":event(2),"envelope":{"source":"host","sequence":2}}])
        );
        assert_eq!(completed_runs[0]["last_sequence"], 2);
        assert_eq!(completed_runs[0]["status"], "succeeded");
        assert_eq!(completed_runs[0]["started_at"], event(1).emitted_at);
        assert_eq!(completed_runs[0]["finished_at"], event(2).emitted_at);
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        assert_eq!(
            runtime
                .execute_received_command(envelope("finish", 2), command(2))
                .unwrap(),
            receipt
        );
        assert_eq!(events(&mut runtime, 0), completed_events);
        assert_eq!(runs(&mut runtime, &path), completed_runs);
        assert_eq!(events(&mut runtime, 2), json!([]));
    }
}

mod workspace_ownership_port_e2e {
    // SPDX-License-Identifier: Apache-2.0

    use nola_application::{
        execute_application_command, ApplicationCommand, SessionRunLease, WorkspaceCheckpoint,
    };
    use nola_store::NolaStore;
    use tempfile::tempdir;

    #[test]
    fn workspace_checkpoint_and_run_lease_survive_application_cold_reopen() {
        let directory = tempdir().unwrap();
        let database = directory.path().join("workspace-ownership.db");
        {
            let mut store = NolaStore::open(&database).unwrap();
            let saved = execute_application_command(
                &mut store,
                ApplicationCommand::SaveWorkspaceCheckpoint {
                    input: WorkspaceCheckpoint {
                        id: "checkpoint-1".into(),
                        root: "/workspace".into(),
                        git_root: "/workspace".into(),
                        revision: "abc123".into(),
                        source: "tool".into(),
                        label: "Before editing".into(),
                        created_at_ms: 10,
                        status: "ready".into(),
                        untracked: vec!["notes.md".into()],
                    },
                },
            )
            .unwrap();
            assert_eq!(saved, serde_json::json!(true));

            let acquired = execute_application_command(
                &mut store,
                ApplicationCommand::AcquireSessionRunLease {
                    input: SessionRunLease {
                        session_id: "conversation-1".into(),
                        owner: "mac-window-1".into(),
                        pid: 42,
                        process_start: "start-token".into(),
                        acquired_at_ms: 11,
                    },
                },
            )
            .unwrap();
            assert_eq!(acquired, serde_json::json!(true));
        }

        let mut reopened = NolaStore::open(&database).unwrap();
        let checkpoints = execute_application_command(
            &mut reopened,
            ApplicationCommand::ListWorkspaceCheckpoints {
                root: "/workspace".into(),
            },
        )
        .unwrap();
        let checkpoints: Vec<WorkspaceCheckpoint> = serde_json::from_value(checkpoints).unwrap();
        assert_eq!(checkpoints.len(), 1);
        assert_eq!(checkpoints[0].untracked, vec!["notes.md"]);

        let lease = execute_application_command(
            &mut reopened,
            ApplicationCommand::GetSessionRunLease {
                session_id: "conversation-1".into(),
            },
        )
        .unwrap();
        let lease: Option<SessionRunLease> = serde_json::from_value(lease).unwrap();
        assert_eq!(lease.unwrap().owner, "mac-window-1");
    }
}
