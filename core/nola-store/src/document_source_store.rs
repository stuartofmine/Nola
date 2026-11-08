// SPDX-License-Identifier: Apache-2.0
//! SQLite document-index repository implementation.
use super::{
    require_id, validate_json, NolaStore, StoreError, StoreResult, StoredDocumentSource,
    StoredDocumentSourceSummary,
};
use rusqlite::{params, Row};

impl NolaStore {
    pub fn replace_document_sources(
        &mut self,
        scope: &str,
        kind: &str,
        sources: &[StoredDocumentSource],
    ) -> StoreResult<usize> {
        require_id("document_source.scope", scope)?;
        require_id("document_source.kind", kind)?;
        if sources.len() > 512 {
            return Err(StoreError::Invariant(
                "document sources are limited to 512 entries per replacement".into(),
            ));
        }
        let mut ids = std::collections::HashSet::new();
        for source in sources {
            validate_document_source(source)?;
            if source.scope != scope || source.kind != kind {
                return Err(StoreError::Invariant(
                    "document source scope and kind must match replacement selector".into(),
                ));
            }
            if !ids.insert(source.id.as_str()) {
                return Err(StoreError::Invariant(format!(
                    "duplicate document source ID: {}",
                    source.id
                )));
            }
        }
        let transaction = self.write_transaction()?;
        transaction.execute(
            "DELETE FROM document_sources WHERE scope=?1 AND kind=?2",
            params![scope, kind],
        )?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO document_sources(
                    id,scope,uri,kind,mime_type,sha256,byte_count,title,text_content,
                    metadata_json,created_at_ms,updated_at_ms
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            )?;
            for source in sources {
                statement.execute(params![
                    source.id,
                    source.scope,
                    source.uri,
                    source.kind,
                    source.mime_type,
                    source.sha256,
                    source.byte_count,
                    source.title,
                    source.text_content,
                    source.metadata_json,
                    source.created_at_ms,
                    source.updated_at_ms,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(sources.len())
    }

    pub fn list_document_sources(
        &self,
        scope: &str,
        kind: Option<&str>,
    ) -> StoreResult<Vec<StoredDocumentSource>> {
        require_id("document_source.scope", scope)?;
        let mut statement = self.connection.prepare(
            "SELECT id,scope,uri,kind,mime_type,sha256,byte_count,title,text_content,
                    metadata_json,created_at_ms,updated_at_ms
             FROM document_sources WHERE scope=?1 AND (?2 IS NULL OR kind=?2)
             ORDER BY updated_at_ms DESC,id ASC",
        )?;
        let rows = statement.query_map(params![scope, kind], document_source_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn search_document_sources(
        &self,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        limit: usize,
    ) -> StoreResult<Vec<StoredDocumentSourceSummary>> {
        require_id("document_source.scope", scope)?;
        let pattern = query.and_then(document_like_pattern);
        let mut statement = self.connection.prepare(
            "SELECT id,scope,uri,kind,mime_type,sha256,byte_count,title,
                    substr(text_content,1,500),
                    CASE WHEN length(CAST(metadata_json AS BLOB))<=?5 THEN metadata_json ELSE '{}' END,
                    created_at_ms,updated_at_ms,length(CAST(metadata_json AS BLOB))>?5
             FROM document_sources
             WHERE scope=?1 AND (?2 IS NULL OR kind=?2)
               AND (?3 IS NULL OR COALESCE(title,'') LIKE ?3 ESCAPE '!'
                    OR COALESCE(text_content,'') LIKE ?3 ESCAPE '!'
                    OR metadata_json LIKE ?3 ESCAPE '!')
             ORDER BY updated_at_ms DESC,id ASC LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![
                scope,
                kind,
                pattern,
                limit.clamp(1, 100) as i64,
                nola_store_port::document_source::DOCUMENT_SUMMARY_METADATA_MAX_BYTES as i64
            ],
            document_source_summary_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

fn document_source_row(row: &Row<'_>) -> rusqlite::Result<StoredDocumentSource> {
    Ok(StoredDocumentSource {
        id: row.get(0)?,
        scope: row.get(1)?,
        uri: row.get(2)?,
        kind: row.get(3)?,
        mime_type: row.get(4)?,
        sha256: row.get(5)?,
        byte_count: row.get(6)?,
        title: row.get(7)?,
        text_content: row.get(8)?,
        metadata_json: row.get(9)?,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
    })
}

fn document_source_summary_row(row: &Row<'_>) -> rusqlite::Result<StoredDocumentSourceSummary> {
    Ok(StoredDocumentSourceSummary {
        id: row.get(0)?,
        scope: row.get(1)?,
        uri: row.get(2)?,
        kind: row.get(3)?,
        mime_type: row.get(4)?,
        sha256: row.get(5)?,
        byte_count: row.get(6)?,
        title: row.get(7)?,
        preview: row.get(8)?,
        metadata_json: row.get(9)?,
        metadata_truncated: row.get(12)?,
        created_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
    })
}

fn validate_document_source(source: &StoredDocumentSource) -> StoreResult<()> {
    require_id("document_source.id", &source.id)?;
    require_id("document_source.scope", &source.scope)?;
    require_id("document_source.uri", &source.uri)?;
    require_id("document_source.kind", &source.kind)?;
    if source.id.chars().count() > 256 {
        return Err(StoreError::Invariant(
            "document source ID exceeds 256 characters".into(),
        ));
    }
    if source
        .text_content
        .as_ref()
        .is_some_and(|content| content.chars().count() > 2_000_000)
    {
        return Err(StoreError::Invariant(
            "document source content exceeds 2000000 characters".into(),
        ));
    }
    validate_json("document_source.metadata_json", &source.metadata_json)
}

fn document_like_pattern(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(format!(
        "%{}%",
        value
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_")
    ))
}
