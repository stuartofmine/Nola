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
