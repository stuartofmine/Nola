use chrono::Utc;
use nola_application::{
    ApplicationCommand, DocumentSource, DocumentSourceCommand, DocumentSourceSummary,
};
use serde_json::{json, Value};
use std::collections::HashSet;

use super::NolaStore;

impl NolaStore {
    pub fn search_documents(
        &self,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let summaries: Vec<DocumentSourceSummary> =
            crate::application_port::CliApplicationPort::persistent(&self.path)?.execute(
                ApplicationCommand::DocumentSource {
                    command: DocumentSourceCommand::Search {
                        scope: scope.to_string(),
                        query: query.map(str::to_string),
                        kind: kind.map(str::to_string),
                        limit,
                    },
                },
            )?;
        summaries.into_iter().map(summary_json).collect()
    }

    pub fn load_web_uploaded_documents(&self) -> Result<Vec<Value>, String> {
        let sources: Vec<DocumentSource> = crate::application_port::CliApplicationPort::persistent(
            &self.path,
        )?
        .execute(ApplicationCommand::DocumentSource {
            command: DocumentSourceCommand::List {
                scope: "web:local".to_string(),
                kind: Some("uploaded_document".to_string()),
            },
        })?;
        sources.into_iter().map(uploaded_document_json).collect()
    }

    pub fn replace_web_uploaded_documents(&mut self, documents: &[Value]) -> Result<(), String> {
        validate_documents(documents)?;
        let now = Utc::now().timestamp_millis();
        let sources = documents
            .iter()
            .enumerate()
            .map(|(index, document)| document_source(document, index, now))
            .collect::<Result<Vec<_>, _>>()?;
        let _: usize = crate::application_port::CliApplicationPort::persistent(&self.path)?
            .execute(ApplicationCommand::DocumentSource {
                command: DocumentSourceCommand::Replace {
                    scope: "web:local".to_string(),
                    kind: "uploaded_document".to_string(),
                    inputs: sources,
                },
            })?;
        Ok(())
    }
}

fn document_source(document: &Value, index: usize, now: i64) -> Result<DocumentSource, String> {
    // Do not clone the body into metadata: search summaries return metadata,
    // and a second body there defeats both canonical ownership and IPC bounds.
    let metadata = document
        .as_object()
        .ok_or("uploaded document must be an object")?
        .iter()
        .filter(|(key, _)| key.as_str() != "content")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let id = web_document_id(document, index);
    let title = document
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Document");
    let uri = document.get("uri").and_then(Value::as_str).unwrap_or(title);
    Ok(DocumentSource {
        id,
        scope: "web:local".to_string(),
        uri: uri.to_string(),
        kind: "uploaded_document".to_string(),
        mime_type: document
            .get("mime")
            .and_then(Value::as_str)
            .map(str::to_string),
        sha256: document
            .get("sha256")
            .and_then(Value::as_str)
            .map(str::to_string),
        byte_count: document.get("size").and_then(Value::as_i64),
        title: Some(title.to_string()),
        text_content: document
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string),
        metadata: Value::Object(metadata),
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn uploaded_document_json(source: DocumentSource) -> Result<Value, String> {
    let Value::Object(mut metadata) = source.metadata else {
        return Err("uploaded document metadata must be an object".into());
    };
    let content = source
        .text_content
        .ok_or("uploaded document canonical content is missing")?;
    metadata.insert("content".into(), Value::String(content));
    Ok(Value::Object(metadata))
}

fn summary_json(summary: DocumentSourceSummary) -> Result<Value, String> {
    Ok(json!({
        "id": summary.id,
        "scope": summary.scope,
        "uri": summary.uri,
        "kind": summary.kind,
        "mime": summary.mime_type,
        "sha256": summary.sha256,
        "sizeBytes": summary.byte_count,
        "title": summary.title,
        "preview": summary.preview,
        "metadataTruncated": summary.metadata_truncated,
        "metadata": summary.metadata,
        "createdAt": summary.created_at_ms,
        "updatedAt": summary.updated_at_ms,
    }))
}

fn web_document_id(document: &Value, index: usize) -> String {
    document
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("web-doc-{value}"))
        .unwrap_or_else(|| format!("web-doc-{index}"))
}

fn validate_documents(documents: &[Value]) -> Result<(), String> {
    if documents.len() > 512 {
        return Err("uploaded documents are limited to 512 entries".to_string());
    }
    let mut ids = HashSet::new();
    for (index, document) in documents.iter().enumerate() {
        let id = web_document_id(document, index);
        if id.chars().count() > 256 {
            return Err(format!(
                "uploaded document ID at index {index} exceeds 256 characters"
            ));
        }
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate uploaded document ID: {id}"));
        }
        for key in ["name", "content"] {
            if document
                .get(key)
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(format!(
                    "uploaded document at index {index} requires canonical field {key}"
                ));
            }
        }
        if document
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| content.chars().count() > 2_000_000)
        {
            return Err(format!(
                "uploaded document content at index {index} exceeds 2000000 characters"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{document_source, uploaded_document_json, validate_documents};
    use serde_json::json;

    #[test]
    fn uploaded_document_payload_rejects_resolved_id_collisions() {
        let documents = vec![
            json!({"id":"1", "name":"One", "content":"First"}),
            json!({"name":"Two", "content":"Second"}),
        ];
        assert!(validate_documents(&documents).is_err());
    }

    #[test]
    fn uploaded_document_payload_rejects_removed_field_aliases() {
        let documents = vec![json!({"id":"old", "title":"Old", "text":"Old body"})];
        assert!(validate_documents(&documents).is_err());
    }

    #[test]
    fn uploaded_document_large_body_has_one_persisted_owner_and_round_trips() {
        let document = json!({
            "id":"large", "name":"Large document", "content":"x".repeat(2_000_000),
            "mime":"text/plain", "size":2_000_000, "enabled":true,
            "labels":["reference"]
        });
        let source = document_source(&document, 0, 1).unwrap();
        assert!(source.metadata.get("content").is_none());
        assert!(serde_json::to_vec(&source.metadata).unwrap().len() < 512);
        assert_eq!(source.text_content.as_ref().unwrap().len(), 2_000_000);
        let encoded = serde_json::to_vec(&source).unwrap();
        assert!(
            encoded.len() < 2_001_024,
            "body was duplicated in the wire record"
        );
        let decoded = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(uploaded_document_json(decoded).unwrap(), document);
    }

    #[test]
    fn uploaded_document_read_never_substitutes_metadata_for_canonical_content() {
        let mut source = document_source(
            &json!({"id":"one", "name":"One", "content":"canonical"}),
            0,
            1,
        )
        .unwrap();
        source.metadata["content"] = json!("not canonical");
        assert_eq!(
            uploaded_document_json(source.clone()).unwrap()["content"],
            "canonical"
        );
        source.text_content = None;
        assert!(uploaded_document_json(source).is_err());
    }
}
