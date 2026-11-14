// SPDX-License-Identifier: Apache-2.0
//! Rebuildable, owner-scoped FTS5 BM25 candidate index for `memory_records`.
//! Canonical `memory_records` columns remain authoritative for ownership,
//! soft-deletion, freshness, and returned record payloads.

use super::*;
use crate::memory_store::{memory_record_row, MEMORY_QUALIFIED_COLUMNS};
use std::collections::BTreeSet;

const MAX_MEMORY_QUERY_TERMS: usize = 32;
const INDEX_BATCH_LIMIT: i64 = 256;
const MAX_INDEX_BATCHES_PER_QUERY: usize = 16;
const MAX_INDEX_BYTES_PER_QUERY: usize = 4 * 1024 * 1024;

pub(super) fn memory_document_terms(
    content: &str,
    summary: Option<&str>,
    tags_json: &str,
) -> String {
    let mut tokens = crate::transcript_index::terms(content);
    if let Some(summary) = summary.filter(|value| !value.trim().is_empty()) {
        tokens.extend(crate::transcript_index::terms(summary));
    }
    if let Ok(serde_json::Value::Array(items)) =
        serde_json::from_str::<serde_json::Value>(tags_json)
    {
        for item in items {
            if let Some(tag) = item.as_str() {
                tokens.extend(crate::transcript_index::terms(tag));
            }
        }
    }
    tokens.join(" ")
}

fn memory_query_expression(query: &str) -> Option<String> {
    let mut query_terms = crate::transcript_index::terms(query);
    let mut seen = BTreeSet::new();
    query_terms.retain(|term| seen.insert(term.clone()));
    if query_terms.len() > MAX_MEMORY_QUERY_TERMS {
        query_terms.sort_by(|left, right| {
            right
                .chars()
                .count()
                .cmp(&left.chars().count())
                .then_with(|| left.cmp(right))
        });
        query_terms.truncate(MAX_MEMORY_QUERY_TERMS);
    }
    if query_terms.is_empty() {
        return None;
    }
    Some(
        query_terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

fn memory_index_schema_ready_on(connection: &Connection) -> StoreResult<bool> {
    connection
        .query_row(
            "SELECT COUNT(*)=4 FROM sqlite_master WHERE name IN (
                'memory_index_heads',
                'memory_bm25',
                'memory_index_delete',
                'memory_index_invalidate'
            )",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

pub(super) fn sync_memory_bm25_on_write(
    transaction: &Connection,
    record: &StoredMemoryRecord,
    revision: i64,
) -> StoreResult<()> {
    if !memory_index_schema_ready_on(transaction)? {
        return Ok(());
    }
    transaction.execute(
        "DELETE FROM memory_bm25 WHERE memory_id=?1",
        params![record.id],
    )?;
    if record.deleted_at_ms.is_some() {
        transaction.execute(
            "DELETE FROM memory_index_heads WHERE memory_id=?1",
            params![record.id],
        )?;
        return Ok(());
    }
    let indexed_terms =
        memory_document_terms(&record.content, record.summary.as_deref(), &record.tags_json);
    let stamp = format!("{revision}:{}", record.updated_at_ms);
    transaction.execute(
        "INSERT INTO memory_bm25(memory_id, owner_json, namespace_json, scope, terms)
         VALUES(?1, ?2, ?3, ?4, ?5)",
        params![
            record.id,
            record.owner_json,
            record.namespace_json,
            record.scope,
            indexed_terms
        ],
    )?;
    transaction.execute(
        "INSERT INTO memory_index_heads(memory_id, stamp) VALUES(?1, ?2)
         ON CONFLICT(memory_id) DO UPDATE SET stamp=excluded.stamp",
        params![record.id, stamp],
    )?;
    Ok(())
}

impl NolaStore {
    fn memory_index_schema_ready(&self) -> StoreResult<bool> {
        memory_index_schema_ready_on(&self.connection)
    }

    pub(super) fn ranked_memory_candidates(
        &self,
        owner_json: &str,
        namespace_json: &str,
        scope: &str,
        query: &str,
        kind: Option<&str>,
        now_iso: &str,
        limit: usize,
    ) -> StoreResult<Vec<(StoredMemoryRecord, f64)>> {
        validate_json("memory.namespace_json", namespace_json)?;
        require_id("memory.scope", scope)?;
        let Some(expression) = memory_query_expression(query) else {
            return Ok(Vec::new());
        };

        let schema_ready = self.memory_index_schema_ready()?;
        if self.is_read_only() {
            if !schema_ready {
                return Ok(Vec::new());
            }
            let has_indexed_rows: bool = self.connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM memory_bm25
                    WHERE owner_json=?1 AND namespace_json=?2 AND scope=?3
                )",
                params![owner_json, namespace_json, scope],
                |row| row.get(0),
            )?;
            if !has_indexed_rows {
                return Ok(Vec::new());
            }
        } else {
            let needs_indexing = if schema_ready {
                self.connection.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM memory_records m
                        WHERE m.owner_json=?1 AND m.namespace_json=?2 AND m.scope=?3
                          AND m.deleted_at_ms IS NULL
                          AND NOT EXISTS(
                              SELECT 1 FROM memory_index_heads h
                              WHERE h.memory_id=m.id
                                AND h.stamp=CAST(m.revision AS TEXT)||':'||CAST(m.updated_at_ms AS TEXT)
                          )
                    )",
                    params![owner_json, namespace_json, scope],
                    |row| row.get(0),
                )?
            } else {
                true
            };

            if needs_indexing {
                let transaction = if self.connection.is_autocommit() {
                    Some(self.connection.unchecked_transaction()?)
                } else {
                    None
                };
                if !schema_ready {
                    self.connection
                        .execute_batch(include_str!("memory_index_schema.sql"))?;
                }
                let mut indexed_bytes = 0usize;
                for _ in 0..MAX_INDEX_BATCHES_PER_QUERY {
                    let mut pending = self.connection.prepare(
                        "SELECT m.id, m.content, m.summary, m.tags_json,
                                CAST(m.revision AS TEXT)||':'||CAST(m.updated_at_ms AS TEXT) AS stamp
                         FROM memory_records m
                         WHERE m.owner_json=?1 AND m.namespace_json=?2 AND m.scope=?3
                           AND m.deleted_at_ms IS NULL
                           AND NOT EXISTS(
                               SELECT 1 FROM memory_index_heads h
                               WHERE h.memory_id=m.id
                                 AND h.stamp=CAST(m.revision AS TEXT)||':'||CAST(m.updated_at_ms AS TEXT)
                           )
                         ORDER BY m.importance DESC, m.confidence DESC, m.updated_at_ms DESC, m.id DESC
                         LIMIT ?4",
                    )?;
                    let rows = pending
                        .query_map(
                            params![owner_json, namespace_json, scope, INDEX_BATCH_LIMIT],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, Option<String>>(2)?,
                                    row.get::<_, String>(3)?,
                                    row.get::<_, String>(4)?,
                                ))
                            },
                        )?
                        .collect::<Result<Vec<_>, _>>()?;
                    drop(pending);
                    if rows.is_empty() {
                        break;
                    }
                    let mut hit_budget = false;
                    for (memory_id, content, summary, tags_json, stamp) in rows {
                        let entry_bytes = content.len()
                            + summary.as_deref().map_or(0, str::len)
                            + tags_json.len();
                        indexed_bytes = indexed_bytes.saturating_add(entry_bytes);
                        let indexed_terms =
                            memory_document_terms(&content, summary.as_deref(), &tags_json);
                        self.connection.execute(
                            "DELETE FROM memory_bm25 WHERE memory_id=?1",
                            params![memory_id],
                        )?;
                        self.connection.execute(
                            "INSERT INTO memory_bm25(memory_id, owner_json, namespace_json, scope, terms)
                             VALUES(?1, ?2, ?3, ?4, ?5)",
                            params![memory_id, owner_json, namespace_json, scope, indexed_terms],
                        )?;
                        self.connection.execute(
                            "INSERT INTO memory_index_heads(memory_id, stamp) VALUES(?1, ?2)
                             ON CONFLICT(memory_id) DO UPDATE SET stamp=excluded.stamp",
                            params![memory_id, stamp],
                        )?;
                        if indexed_bytes >= MAX_INDEX_BYTES_PER_QUERY {
                            hit_budget = true;
                            break;
                        }
                    }
                    if hit_budget {
                        break;
                    }
                }
                if let Some(transaction) = transaction {
                    transaction.commit()?;
                }
            }
        }

        let bounded_limit = limit.clamp(1, 200) as i64;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MEMORY_QUALIFIED_COLUMNS}, bm25(memory_bm25)
             FROM memory_bm25
             JOIN memory_records m ON m.id = memory_bm25.memory_id
             WHERE memory_bm25 MATCH ?1
               AND memory_bm25.owner_json = ?2
               AND memory_bm25.namespace_json = ?3
               AND memory_bm25.scope = ?4
               AND m.owner_json = ?2
               AND m.namespace_json = ?3
               AND m.scope = ?4
               AND m.deleted_at_ms IS NULL
               AND (?5 IS NULL OR m.kind = ?5)
               AND (m.expires_at IS NULL OR julianday(m.expires_at) > julianday(?6))
             ORDER BY bm25(memory_bm25) ASC,
                      m.importance DESC,
                      m.confidence DESC,
                      m.updated_at_ms DESC,
                      m.id DESC
             LIMIT ?7"
        ))?;
        let rows = statement.query_map(
            params![
                expression,
                owner_json,
                namespace_json,
                scope,
                kind,
                now_iso,
                bounded_limit
            ],
            |row| Ok((memory_record_row(row)?, row.get::<_, f64>(19)?)),
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_store_port::memory::MemoryReadRepository;

    const LOCAL_OWNER: &str = r#"{"kind":"profile","profile_id":"local"}"#;
    const OTHER_OWNER: &str = r#"{"kind":"profile","profile_id":"guest-b"}"#;
    const PERSONAL_NS: &str = r#"{"kind":"personal"}"#;
    const NOW_ISO: &str = "2026-09-10T00:00:00.000Z";

    fn fixture_memory(
        id: &str,
        owner_json: &str,
        content: &str,
        summary: Option<&str>,
        tags_json: &str,
        updated_at_ms: i64,
    ) -> StoredMemoryRecord {
        StoredMemoryRecord {
            id: id.into(),
            owner_json: owner_json.into(),
            namespace_json: PERSONAL_NS.into(),
            scope: "global".into(),
            kind: "preference".into(),
            content: content.into(),
            summary: summary.map(str::to_string),
            tags_json: tags_json.into(),
            confidence: 0.85,
            importance: 0.75,
            sensitivity: None,
            retention_json: r#"{"mode":"until_superseded"}"#.into(),
            expires_at: None,
            supersedes_json: "[]".into(),
            provenance_json: r#"{"kind":"explicit_user","source":"test","session_id":"s1","turn_id":null,"item_ids":[],"observed_at":null}"#.into(),
            created_at_ms: updated_at_ms,
            updated_at_ms,
            last_used_at_ms: None,
            deleted_at_ms: None,
        }
    }

    #[test]
    fn multi_term_natural_language_bm25_ranks_strongest_overlap_first_and_boosts_exact() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store
            .upsert_memory_record(&fixture_memory(
                "single-term",
                LOCAL_OWNER,
                "PostgreSQL backups run nightly at midnight.",
                None,
                "[]",
                300,
            ))
            .unwrap();
        store
            .upsert_memory_record(&fixture_memory(
                "multi-term",
                LOCAL_OWNER,
                "Configure PostgreSQL connection pooler statement timeout to 15 seconds.",
                Some("Database reliability policy"),
                "[]",
                100,
            ))
            .unwrap();

        let recalled = MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("What is our PostgreSQL connection timeout policy?"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap();
        let ids = recalled.iter().map(|r| r.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids, vec!["multi-term", "single-term"]);

        // Exact substring match receives highest priority boost even if lower importance.
        let mut exact_low_importance = fixture_memory(
            "exact-phrase",
            LOCAL_OWNER,
            "Always check connection timeout before retrying.",
            None,
            "[]",
            50,
        );
        exact_low_importance.importance = 0.2;
        store.upsert_memory_record(&exact_low_importance).unwrap();

        let boosted = MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("connection timeout"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap();
        assert_eq!(boosted[0].id, "exact-phrase");
    }

    #[test]
    fn cjk_bigram_and_tags_json_recall_work_via_fts5_bm25() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store
            .upsert_memory_record(&fixture_memory(
                "cjk-strong",
                LOCAL_OWNER,
                "本地向量与全文检索需要优先保证缓存命中率与确定性排序。",
                None,
                "[]",
                100,
            ))
            .unwrap();
        store
            .upsert_memory_record(&fixture_memory(
                "cjk-weak",
                LOCAL_OWNER,
                "今天调试命中了一个边界条件。",
                None,
                "[]",
                200,
            ))
            .unwrap();
        store
            .upsert_memory_record(&fixture_memory(
                "tagged-only",
                LOCAL_OWNER,
                "Rotate staging credentials every 30 days.",
                None,
                r#"["k8s_ingress", "零信任网关"]"#,
                150,
            ))
            .unwrap();

        let cjk = MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("如何提升缓存命中表现"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap();
        assert!(!cjk.is_empty());
        assert_eq!(cjk[0].id, "cjk-strong");

        let by_identifier_tag = MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("k8s_ingress"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap();
        assert_eq!(
            by_identifier_tag.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["tagged-only"]
        );

        let by_cjk_tag = MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("零信任网关配置"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap();
        assert_eq!(
            by_cjk_tag.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["tagged-only"]
        );
    }

    #[test]
    fn update_and_soft_delete_invalidate_bm25_and_preserve_owner_isolation_and_supersession() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store
            .upsert_memory_record(&fixture_memory(
                "mutable-mem",
                LOCAL_OWNER,
                "Use legacy_alpha_endpoint for telemetry.",
                None,
                r#"["old_tag_token"]"#,
                100,
            ))
            .unwrap();
        store
            .upsert_memory_record(&fixture_memory(
                "foreign-owner-mem",
                OTHER_OWNER,
                "Use legacy_alpha_endpoint and secret_foreign_token.",
                None,
                "[]",
                200,
            ))
            .unwrap();

        // Initial recall populates BM25 index for LOCAL_OWNER and isolates OTHER_OWNER.
        let initial = MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("legacy_alpha_endpoint secret_foreign_token"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap();
        assert_eq!(
            initial.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["mutable-mem"]
        );

        // Update content + tags_json: old tokens must be evicted by trigger and new tokens indexed.
        let mut updated = fixture_memory(
            "mutable-mem",
            LOCAL_OWNER,
            "Use modern_beta_collector for telemetry.",
            None,
            r#"["new_tag_token"]"#,
            300,
        );
        updated.created_at_ms = 100;
        store.upsert_memory_record(&updated).unwrap();

        assert!(MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("legacy_alpha_endpoint"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap()
        .is_empty());
        assert!(MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("old_tag_token"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap()
        .is_empty());
        assert_eq!(
            MemoryReadRepository::recall_memory_records(
                &store,
                LOCAL_OWNER,
                PERSONAL_NS,
                "global",
                Some("modern_beta_collector new_tag_token"),
                None,
                NOW_ISO,
                10,
            )
            .unwrap()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
            vec!["mutable-mem"]
        );

        // Supersession chain completion: successor without query terms is still appended.
        let mut successor = fixture_memory(
            "successor-mem",
            LOCAL_OWNER,
            "Telemetry collector is disabled completely.",
            None,
            "[]",
            400,
        );
        successor.supersedes_json = r#"["mutable-mem"]"#.into();
        store.upsert_memory_record(&successor).unwrap();

        let with_chain = MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("new_tag_token"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap();
        assert_eq!(
            with_chain.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["mutable-mem", "successor-mem"]
        );

        // Soft-delete invalidates BM25 index immediately.
        assert!(store.delete_memory_record("mutable-mem", 500).unwrap());
        assert!(MemoryReadRepository::recall_memory_records(
            &store,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("new_tag_token"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn pre_existing_epoch_one_database_lazily_builds_memory_fts_without_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing_epoch1.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store
            .upsert_memory_record(&fixture_memory(
                "legacy-row",
                LOCAL_OWNER,
                "Pre-existing epoch 1 record about deterministic_fts_backfill.",
                None,
                r#"["lazy_index"]"#,
                100,
            ))
            .unwrap();
        // Simulate an already-initialized epoch=1 database created prior to memory_bm25 DDL.
        store
            .connection
            .execute_batch(
                "DROP TRIGGER memory_index_delete;
                 DROP TRIGGER memory_index_invalidate;
                 DROP TABLE memory_bm25;
                 DROP TABLE memory_index_heads;",
            )
            .unwrap();
        drop(store);

        let reopened = NolaStore::open(&path).unwrap();
        assert!(!reopened.memory_index_schema_ready().unwrap());
        let recalled = MemoryReadRepository::recall_memory_records(
            &reopened,
            LOCAL_OWNER,
            PERSONAL_NS,
            "global",
            Some("deterministic_fts_backfill lazy_index"),
            None,
            NOW_ISO,
            10,
        )
        .unwrap();
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].id, "legacy-row");
        assert!(reopened.memory_index_schema_ready().unwrap());
    }
}
