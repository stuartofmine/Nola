// SPDX-License-Identifier: Apache-2.0
//! Document index persistence without a concrete database dependency.
use serde::{Deserialize, Serialize};

pub const DOCUMENT_SUMMARY_METADATA_MAX_BYTES: usize = 4096;

pub trait DocumentSourceRepository {
    type Error: std::fmt::Display;

    /// Atomically replace exactly one scope/kind partition. Reject invalid or
    /// conflicting identities without deleting the old partition or other data.
    /// Preserve the caller's transaction and return only the committed count.
    /// A mutation acknowledgement must not echo all document bodies.
    fn replace_document_sources(
        &mut self,
        scope: &str,
        kind: &str,
        inputs: &[StoredDocumentSource],
    ) -> Result<usize, Self::Error>;
    fn list_document_sources(
        &self,
        scope: &str,
        kind: Option<&str>,
    ) -> Result<Vec<StoredDocumentSource>, Self::Error>;
    /// Return summaries, not full bodies. Query text is literal caller input;
    /// escaping for the underlying query engine belongs to the adapter.
    fn search_document_sources(
        &self,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredDocumentSourceSummary>, Self::Error>;
}

/// Composition port that lends only document-index persistence for one use case.
pub trait DocumentSourceRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: DocumentSourceRepository<Error = Self::Error>
    where
        Self: 'a;

    fn document_source_repository(&mut self) -> Self::Repository<'_>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDocumentSource {
    pub id: String,
    pub scope: String,
    pub uri: String,
    pub kind: String,
    pub mime_type: Option<String>,
    pub sha256: Option<String>,
    pub byte_count: Option<i64>,
    pub title: Option<String>,
    pub text_content: Option<String>,
    pub metadata_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredDocumentSourceSummary {
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
    pub metadata_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}
