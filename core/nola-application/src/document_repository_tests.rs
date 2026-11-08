// SPDX-License-Identifier: Apache-2.0
use crate::document_source::{execute_document_source_command, DocumentSourceCommand};
use nola_store_port::document_source::*;

struct SearchRepository;
impl DocumentSourceRepository for SearchRepository {
    type Error = String;
    fn replace_document_sources(
        &mut self,
        _: &str,
        _: &str,
        _: &[StoredDocumentSource],
    ) -> Result<usize, String> {
        panic!("unexpected write")
    }
    fn list_document_sources(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<StoredDocumentSource>, String> {
        panic!("unexpected full document load")
    }
    fn search_document_sources(
        &self,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredDocumentSourceSummary>, String> {
        assert_eq!(scope, "conversation:1");
        assert_eq!(query, Some("  预算 %_\\ Current  "));
        assert_eq!(kind, None);
        assert_eq!(limit, 8);
        Err("repository unavailable".into())
    }
}

#[test]
fn search_preserves_input_and_uses_only_summary_repository_without_sqlite() {
    let error = execute_document_source_command(
        &mut SearchRepository,
        DocumentSourceCommand::Search {
            scope: "conversation:1".into(),
            query: Some("  预算 %_\\ Current  ".into()),
            kind: None,
            limit: 8,
        },
    )
    .unwrap_err();
    assert_eq!(error, "repository unavailable");
}

#[test]
fn malformed_replace_is_rejected_before_reaching_the_repository() {
    let error = execute_document_source_command(
        &mut SearchRepository,
        DocumentSourceCommand::Replace {
            scope: " ".into(),
            kind: "uploaded_document".into(),
            inputs: vec![],
        },
    )
    .unwrap_err();
    assert!(error.contains("scope and kind"));
}
