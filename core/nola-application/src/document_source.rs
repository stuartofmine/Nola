// SPDX-License-Identifier: Apache-2.0
//! Document-source index use-case boundary.

use nola_store_port::document_source::{
    DocumentSourceRepository, StoredDocumentSource, StoredDocumentSourceSummary,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSource {
    pub id: String,
    pub scope: String,
    pub uri: String,
    pub kind: String,
    pub mime_type: Option<String>,
    pub sha256: Option<String>,
    pub byte_count: Option<i64>,
    pub title: Option<String>,
    pub text_content: Option<String>,
    pub metadata: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentSourceSummary {
    pub id: String,
    pub scope: String,
    pub uri: String,
    pub kind: String,
    pub mime_type: Option<String>,
    pub sha256: Option<String>,
    pub byte_count: Option<i64>,
    pub title: Option<String>,
    pub preview: Option<String>,
    pub metadata_truncated: bool,
    pub metadata: Value,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DocumentSourceCommand {
    /// Returns the committed document count, never a copy of the input bodies.
    Replace {
        scope: String,
        kind: String,
        inputs: Vec<DocumentSource>,
    },
    List {
        scope: String,
        kind: Option<String>,
    },
    Search {
        scope: String,
        query: Option<String>,
        kind: Option<String>,
        limit: usize,
    },
}

pub(crate) fn execute_document_source_command<R: DocumentSourceRepository + ?Sized>(
    store: &mut R,
    command: DocumentSourceCommand,
) -> Result<Value, String> {
    match command {
        DocumentSourceCommand::Replace {
            scope,
            kind,
            inputs,
        } => {
            if scope.trim().is_empty() || kind.trim().is_empty() {
                return Err("document source scope and kind are required".into());
            }
            let mut identities = std::collections::HashSet::new();
            for input in &inputs {
                if input.scope != scope || input.kind != kind {
                    return Err("document source replacement cannot cross scope or kind".into());
                }
                if input.id.trim().is_empty() || !identities.insert(input.id.as_str()) {
                    return Err(
                        "document source replacement requires unique nonempty identities".into(),
                    );
                }
            }
            let inputs = inputs
                .into_iter()
                .map(stored_document_source)
                .collect::<Result<Vec<_>, _>>()?;
            let replaced = store
                .replace_document_sources(&scope, &kind, &inputs)
                .map_err(|error| error.to_string())?;
            serde_json::to_value(replaced)
        }
        DocumentSourceCommand::List { scope, kind } => {
            let sources = store
                .list_document_sources(&scope, kind.as_deref())
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(document_source)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_value(sources)
        }
        DocumentSourceCommand::Search {
            scope,
            query,
            kind,
            limit,
        } => {
            let summaries = store
                .search_document_sources(&scope, query.as_deref(), kind.as_deref(), limit)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(document_source_summary)
                .collect::<Result<Vec<_>, _>>()?;
            serde_json::to_value(summaries)
        }
    }
    .map_err(|error| format!("encode document source result failed: {error}"))
}

fn stored_document_source(source: DocumentSource) -> Result<StoredDocumentSource, String> {
    Ok(StoredDocumentSource {
        id: source.id,
        scope: source.scope,
        uri: source.uri,
        kind: source.kind,
        mime_type: source.mime_type,
        sha256: source.sha256,
        byte_count: source.byte_count,
        title: source.title,
        text_content: source.text_content,
        metadata_json: serde_json::to_string(&source.metadata)
            .map_err(|error| format!("encode document source metadata failed: {error}"))?,
        created_at_ms: source.created_at_ms,
        updated_at_ms: source.updated_at_ms,
    })
}

fn document_source(source: StoredDocumentSource) -> Result<DocumentSource, String> {
    Ok(DocumentSource {
        id: source.id,
        scope: source.scope,
        uri: source.uri,
        kind: source.kind,
        mime_type: source.mime_type,
        sha256: source.sha256,
        byte_count: source.byte_count,
        title: source.title,
        text_content: source.text_content,
        metadata: serde_json::from_str(&source.metadata_json)
            .map_err(|error| format!("decode document source metadata failed: {error}"))?,
        created_at_ms: source.created_at_ms,
        updated_at_ms: source.updated_at_ms,
    })
}

fn document_source_summary(
    summary: StoredDocumentSourceSummary,
) -> Result<DocumentSourceSummary, String> {
    Ok(DocumentSourceSummary {
        id: summary.id,
        scope: summary.scope,
        uri: summary.uri,
        kind: summary.kind,
        mime_type: summary.mime_type,
        sha256: summary.sha256,
        byte_count: summary.byte_count,
        title: summary.title,
        preview: summary.preview,
        metadata_truncated: summary.metadata_truncated,
        metadata: serde_json::from_str(&summary.metadata_json)
            .map_err(|error| format!("decode document source metadata failed: {error}"))?,
        created_at_ms: summary.created_at_ms,
        updated_at_ms: summary.updated_at_ms,
    })
}
