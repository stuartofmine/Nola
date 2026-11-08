// SPDX-License-Identifier: Apache-2.0
//! Narrow document-index adapter sharing the Application-owned transaction.
use crate::{NolaStore, StoreError};
use nola_store_port::document_source::{
    DocumentSourceRepository, DocumentSourceRepositoryProvider, StoredDocumentSource,
    StoredDocumentSourceSummary,
};

/// Borrowed document-index capability.
///
/// ```compile_fail
/// use nola_store::{DocumentSourceRepository, NolaStore};
/// fn requires_documents(_: &mut impl DocumentSourceRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// requires_documents(&mut store);
/// ```
///
/// ```compile_fail
/// use nola_store::NolaStore;
/// use nola_store_port::data_source::DataSourceRepository;
/// fn requires_data_sources(_: &mut impl DataSourceRepository) {}
/// let mut store = NolaStore::open(":memory:").unwrap();
/// let mut documents = store.document_source_repository();
/// requires_data_sources(&mut documents);
/// ```
pub struct SqliteDocumentSourceRepository<'store> {
    store: &'store mut NolaStore,
}

impl NolaStore {
    pub fn document_source_repository(&mut self) -> SqliteDocumentSourceRepository<'_> {
        SqliteDocumentSourceRepository { store: self }
    }
}

impl DocumentSourceRepositoryProvider for NolaStore {
    type Error = StoreError;
    type Repository<'a> = SqliteDocumentSourceRepository<'a>;

    fn document_source_repository(&mut self) -> Self::Repository<'_> {
        NolaStore::document_source_repository(self)
    }
}

impl DocumentSourceRepository for SqliteDocumentSourceRepository<'_> {
    type Error = StoreError;
    fn replace_document_sources(
        &mut self,
        scope: &str,
        kind: &str,
        inputs: &[StoredDocumentSource],
    ) -> Result<usize, Self::Error> {
        self.store.replace_document_sources(scope, kind, inputs)
    }
    fn list_document_sources(
        &self,
        scope: &str,
        kind: Option<&str>,
    ) -> Result<Vec<StoredDocumentSource>, Self::Error> {
        self.store.list_document_sources(scope, kind)
    }
    fn search_document_sources(
        &self,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredDocumentSourceSummary>, Self::Error> {
        self.store
            .search_document_sources(scope, query, kind, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(id: &str) -> StoredDocumentSource {
        StoredDocumentSource {
            id: id.into(),
            scope: "scope-a".into(),
            uri: format!("memory://{id}"),
            kind: "note".into(),
            mime_type: Some("text/plain".into()),
            sha256: None,
            byte_count: Some(4),
            title: Some("Test".into()),
            text_content: Some("body".into()),
            metadata_json: "{}".into(),
            created_at_ms: 10,
            updated_at_ms: 10,
        }
    }

    #[test]
    fn narrow_repository_replacement_joins_owner_unit_of_work() {
        let mut store = NolaStore::open(":memory:").expect("open Store");
        store.begin_write_unit_of_work().expect("begin rollback");
        store
            .document_source_repository()
            .replace_document_sources("scope-a", "note", &[document("document-a")])
            .expect("replace documents");
        store.rollback_write_unit_of_work().expect("rollback");
        assert!(store
            .document_source_repository()
            .list_document_sources("scope-a", None)
            .expect("list after rollback")
            .is_empty());

        store.begin_write_unit_of_work().expect("begin commit");
        store
            .document_source_repository()
            .replace_document_sources("scope-a", "note", &[document("document-a")])
            .expect("replace documents");
        store.commit_write_unit_of_work().expect("commit");
        assert_eq!(
            store
                .document_source_repository()
                .list_document_sources("scope-a", None)
                .expect("list after commit")
                .len(),
            1
        );
    }
}
