// SPDX-License-Identifier: Apache-2.0
//! Memory record persistence and usage ledger operations.

use crate::{
    enqueue_sync_change, require_id, validate_json, NolaStore, StoreError, StoreResult,
    StoredMemoryRecord, StoredMemoryUsageEntry, StoredMemoryUsageStats,
};
use rusqlite::{params, Connection, OptionalExtension, Row};

fn memory_provenance_kind(provenance_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(provenance_json)
        .ok()?
        .get("kind")?
        .as_str()
        .map(ToOwned::to_owned)
}

const MEMORY_COLUMNS: &str = "id,namespace_json,scope,kind,content,summary,tags_json,confidence,importance,sensitivity,retention_json,expires_at,supersedes_json,provenance_json,created_at_ms,updated_at_ms,last_used_at_ms,deleted_at_ms,owner_json";

impl NolaStore {
    pub fn upsert_memory_record(
        &mut self,
        record: &StoredMemoryRecord,
    ) -> StoreResult<StoredMemoryRecord> {
        validate_memory_record(record)?;
        let transaction = self.write_transaction()?;
        let stored = upsert_memory_projection(&transaction, record)?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn save_memory_record_deduplicated(
        &mut self,
        record: &StoredMemoryRecord,
    ) -> StoreResult<StoredMemoryRecord> {
        validate_memory_record(record)?;
        let transaction = self.write_transaction()?;
        let previous_id = validate_memory_candidate_identity(&transaction, record)?;
        let existing_id: Option<String> = transaction
            .query_row(
                "SELECT id FROM memory_records WHERE namespace_json=?1 AND scope=?2 AND kind=?3
             AND content=?4 AND owner_json=?5 AND deleted_at_ms IS NULL ORDER BY updated_at_ms DESC,id DESC LIMIT 1",
                params![
                    record.namespace_json,
                    record.scope,
                    record.kind,
                    record.content,
                    record.owner_json
                ],
                |row| row.get(0),
            )
            .optional()?;
        let mut canonical = record.clone();
        if let Some(id) = existing_id {
            let supersedes = serde_json::from_str::<Vec<String>>(&canonical.supersedes_json)?;
            if supersedes.iter().any(|superseded_id| superseded_id == &id) {
                return Err(StoreError::Conflict(format!(
                    "memory_supersession_identity_conflict: deduplication would make memory {id} supersede itself"
                )));
            }
            canonical.id = id;
            if let Some(existing) = transaction
                .query_row(
                    &format!("SELECT {MEMORY_COLUMNS} FROM memory_records WHERE id=?1"),
                    params![canonical.id],
                    memory_record_row,
                )
                .optional()?
            {
                if existing.updated_at_ms > canonical.updated_at_ms {
                    return Err(StoreError::Invariant(
                        "memory_stale_candidate: duplicate candidate predates the canonical memory"
                            .into(),
                    ));
                }
                // Automatic extraction is intentionally idempotent, but it must
                // not turn an explicit user fact into an agent inference merely
                // because both candidates normalized to the same content. User
                // edits and corrections remain the authoritative provenance;
                // an inferred duplicate only confirms the already stored fact.
                if memory_provenance_kind(&existing.provenance_json).as_deref()
                    == Some("explicit_user")
                    && memory_provenance_kind(&canonical.provenance_json).as_deref()
                        != Some("explicit_user")
                {
                    let mut preserved = existing.clone();
                    preserved.last_used_at_ms =
                        preserved.last_used_at_ms.max(canonical.last_used_at_ms);
                    canonical = preserved;
                }
                canonical.created_at_ms = existing.created_at_ms;
                canonical.last_used_at_ms = canonical.last_used_at_ms.max(existing.last_used_at_ms);
            }
        }
        if let Some(previous) = previous_id.filter(|previous| previous.id == canonical.id) {
            canonical.last_used_at_ms = canonical.last_used_at_ms.max(previous.last_used_at_ms);
        }
        let stored = upsert_memory_projection(&transaction, &canonical)?;
        transaction.commit()?;
        Ok(stored)
    }

    pub fn memory_record(&self, id: &str) -> StoreResult<Option<StoredMemoryRecord>> {
        require_id("memory.id", id)?;
        self.connection.query_row(
            &format!("SELECT {MEMORY_COLUMNS} FROM memory_records WHERE id=?1 AND deleted_at_ms IS NULL"),
            params![id], memory_record_row,
        ).optional().map_err(StoreError::from)
    }

    pub fn memory_record_with_revision(
        &self,
        id: &str,
    ) -> StoreResult<Option<(StoredMemoryRecord, i64)>> {
        require_id("memory.id", id)?;
        self.connection.query_row(
            &format!("SELECT {MEMORY_COLUMNS},revision FROM memory_records WHERE id=?1 AND deleted_at_ms IS NULL"),
            params![id], |row| Ok((memory_record_row(row)?, row.get(19)?)),
        ).optional().map_err(StoreError::from)
    }

    pub fn memory_records_by_ids(&self, ids: &[String]) -> StoreResult<Vec<StoredMemoryRecord>> {
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(record) = self.memory_record(id)? {
                records.push(record);
            }
        }
        Ok(records)
    }

    pub fn list_memory_records(&self, limit: usize) -> StoreResult<Vec<StoredMemoryRecord>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MEMORY_COLUMNS} FROM memory_records WHERE deleted_at_ms IS NULL
             ORDER BY COALESCE(last_used_at_ms,updated_at_ms) DESC,updated_at_ms DESC,id DESC LIMIT ?1"
        ))?;
        let rows = statement.query_map(params![limit.clamp(1, 2_000) as i64], memory_record_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn search_memory_records(
        &self,
        owner_json: &str,
        namespace_json: &str,
        scope: &str,
        query_pattern: Option<&str>,
        kind: Option<&str>,
        now_iso: &str,
        limit: usize,
    ) -> StoreResult<Vec<StoredMemoryRecord>> {
        validate_json("memory.namespace_json", namespace_json)?;
        require_id("memory.scope", scope)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MEMORY_COLUMNS} FROM memory_records
             WHERE namespace_json=?1 AND scope=?2 AND owner_json=?7 AND deleted_at_ms IS NULL
             AND (?3 IS NULL OR kind=?3)
             AND (?4 IS NULL OR content LIKE ?4 ESCAPE '\\' OR COALESCE(summary,'') LIKE ?4 ESCAPE '\\')
             AND (expires_at IS NULL OR julianday(expires_at)>julianday(?5))
             ORDER BY importance DESC,confidence DESC,
                (SELECT COUNT(*) FROM memory_usage_ledger usage WHERE usage.memory_id=memory_records.id) DESC,
                COALESCE(last_used_at_ms,updated_at_ms) DESC,updated_at_ms DESC,id DESC LIMIT ?6"
        ))?;
        let rows = statement.query_map(
            params![
                namespace_json,
                scope,
                kind,
                query_pattern,
                now_iso,
                limit.clamp(1, 50) as i64,
                owner_json
            ],
            memory_record_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn memory_usage_stats(
        &self,
        memory_ids: &[String],
    ) -> StoreResult<Vec<StoredMemoryUsageStats>> {
        let mut result = Vec::with_capacity(memory_ids.len());
        let mut statement = self.connection.prepare(
            "SELECT COUNT(*),MAX(used_at_ms) FROM memory_usage_ledger WHERE memory_id=?1",
        )?;
        for memory_id in memory_ids {
            require_id("memory.id", memory_id)?;
            let (usage_count, last_used_at_ms): (i64, Option<i64>) =
                statement.query_row(params![memory_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            if let Some(last_used_at_ms) = last_used_at_ms {
                result.push(StoredMemoryUsageStats {
                    memory_id: memory_id.clone(),
                    usage_count: usage_count as u64,
                    last_used_at_ms,
                });
            }
        }
        Ok(result)
    }

    /// Loads active records that supersede one candidate. Recall uses this to
    /// complete a bounded successor chain even when the correcting fact does
    /// not contain the original query text.
    pub fn memory_records_superseding(
        &self,
        owner_json: &str,
        namespace_json: &str,
        scope: &str,
        memory_id: &str,
        now_iso: &str,
        limit: usize,
    ) -> StoreResult<Vec<StoredMemoryRecord>> {
        validate_json("memory.namespace_json", namespace_json)?;
        require_id("memory.scope", scope)?;
        require_id("memory.id", memory_id)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MEMORY_COLUMNS} FROM memory_records
             WHERE namespace_json=?1 AND scope=?2 AND owner_json=?6 AND deleted_at_ms IS NULL
             AND (expires_at IS NULL OR julianday(expires_at)>julianday(?3))
             AND EXISTS (
                 SELECT 1 FROM json_each(memory_records.supersedes_json)
                 WHERE json_each.value=?4
             )
             ORDER BY importance DESC,confidence DESC,updated_at_ms DESC,id DESC LIMIT ?5"
        ))?;
        let rows = statement.query_map(
            params![
                namespace_json,
                scope,
                now_iso,
                memory_id,
                limit.clamp(1, 200) as i64,
                owner_json
            ],
            memory_record_row,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn delete_memory_record(&mut self, id: &str, deleted_at_ms: i64) -> StoreResult<bool> {
        require_id("memory.id", id)?;
        let transaction = self.write_transaction()?;
        let changed = transaction.execute(
            "UPDATE memory_records SET deleted_at_ms=?2,updated_at_ms=?2,revision=revision+1
             WHERE id=?1 AND deleted_at_ms IS NULL",
            params![id, deleted_at_ms],
        )? == 1;
        if changed {
            let revision: i64 = transaction.query_row(
                "SELECT revision FROM memory_records WHERE id=?1",
                params![id],
                |row| row.get(0),
            )?;
            enqueue_sync_change(
                &transaction,
                "memory",
                id,
                revision,
                true,
                "null",
                deleted_at_ms,
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }
}

pub(super) const MEMORY_QUALIFIED_COLUMNS: &str = "m.id,m.namespace_json,m.scope,m.kind,m.content,m.summary,m.tags_json,m.confidence,m.importance,m.sensitivity,m.retention_json,m.expires_at,m.supersedes_json,m.provenance_json,m.created_at_ms,m.updated_at_ms,m.last_used_at_ms,m.deleted_at_ms,m.owner_json";

pub(super) fn memory_record_row(row: &Row<'_>) -> rusqlite::Result<StoredMemoryRecord> {
    Ok(StoredMemoryRecord {
        id: row.get(0)?,
        owner_json: row.get(18)?,
        namespace_json: row.get(1)?,
        scope: row.get(2)?,
        kind: row.get(3)?,
        content: row.get(4)?,
        summary: row.get(5)?,
        tags_json: row.get(6)?,
        confidence: row.get(7)?,
        importance: row.get(8)?,
        sensitivity: row.get(9)?,
        retention_json: row.get(10)?,
        expires_at: row.get(11)?,
        supersedes_json: row.get(12)?,
        provenance_json: row.get(13)?,
        created_at_ms: row.get(14)?,
        updated_at_ms: row.get(15)?,
        last_used_at_ms: row.get(16)?,
        deleted_at_ms: row.get(17)?,
    })
}

pub(super) fn validate_memory_record(record: &StoredMemoryRecord) -> StoreResult<()> {
    require_id("memory.id", &record.id)?;
    let owner: nola_domain::memory_owner::MemoryOwner = serde_json::from_str(&record.owner_json)?;
    let namespace: nola_domain::MemoryNamespace = serde_json::from_str(&record.namespace_json)?;
    let scope: nola_domain::MemoryScope = serde_json::from_value(serde_json::json!(record.scope))?;
    owner
        .validate_scope(&namespace, scope)
        .map_err(StoreError::Invariant)?;
    if serde_json::to_string(&owner)? != record.owner_json {
        return Err(StoreError::Invariant(
            "memory owner must use canonical encoding".into(),
        ));
    }
    require_id("memory.scope", &record.scope)?;
    require_id("memory.kind", &record.kind)?;
    if record.content.trim().is_empty() {
        return Err(StoreError::Invariant(
            "memory content must not be empty".into(),
        ));
    }
    if record.content.chars().count() > 100_000 {
        return Err(StoreError::Invariant(
            "memory content exceeds 100000 characters".into(),
        ));
    }
    if !record.confidence.is_finite()
        || !(0.0..=1.0).contains(&record.confidence)
        || !record.importance.is_finite()
        || !(0.0..=1.0).contains(&record.importance)
    {
        return Err(StoreError::Invariant(
            "memory confidence and importance must be between 0 and 1".into(),
        ));
    }
    validate_json("memory.namespace_json", &record.namespace_json)?;
    validate_json("memory.tags_json", &record.tags_json)?;
    validate_json("memory.retention_json", &record.retention_json)?;
    validate_json("memory.supersedes_json", &record.supersedes_json)?;
    validate_json("memory.provenance_json", &record.provenance_json)?;
    Ok(())
}

pub(super) fn apply_memory_usage_entries(
    transaction: &Connection,
    entries: &[StoredMemoryUsageEntry],
) -> StoreResult<()> {
    for entry in entries {
        require_id("memory_usage.usage_id", &entry.usage_id)?;
        require_id("memory_usage.memory_id", &entry.memory_id)?;
        require_id("memory_usage.session_id", &entry.session_id)?;
        require_id("memory_usage.turn_id", &entry.turn_id)?;
        require_id(
            "memory_usage.context_payload_digest",
            &entry.context_payload_digest,
        )?;
        validate_json("memory_usage.citation_ids_json", &entry.citation_ids_json)?;
        if entry.used_at_ms < 0 {
            return Err(StoreError::Invariant(
                "memory usage timestamp must not be negative".to_string(),
            ));
        }
        let memory_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_records WHERE id=?1 AND deleted_at_ms IS NULL)",
            params![entry.memory_id],
            |row| row.get(0),
        )?;
        if !memory_exists {
            return Err(StoreError::Conflict(format!(
                "memory usage references missing active memory {}",
                entry.memory_id
            )));
        }
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO memory_usage_ledger(
                usage_id,memory_id,session_id,turn_id,used_at_ms,
                context_payload_digest,citation_ids_json,created_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?5)",
            params![
                entry.usage_id,
                entry.memory_id,
                entry.session_id,
                entry.turn_id,
                entry.used_at_ms,
                entry.context_payload_digest,
                entry.citation_ids_json,
            ],
        )?;
        if inserted == 1 {
            transaction.execute(
                "UPDATE memory_records
                 SET last_used_at_ms=MAX(COALESCE(last_used_at_ms,?2),?2)
                 WHERE id=?1 AND deleted_at_ms IS NULL",
                params![entry.memory_id, entry.used_at_ms],
            )?;
        } else {
            let existing: (String, String, String, i64, String, String) = transaction.query_row(
                "SELECT memory_id,session_id,turn_id,used_at_ms,
                        context_payload_digest,citation_ids_json
                 FROM memory_usage_ledger WHERE usage_id=?1",
                params![entry.usage_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )?;
            let incoming = (
                entry.memory_id.clone(),
                entry.session_id.clone(),
                entry.turn_id.clone(),
                entry.used_at_ms,
                entry.context_payload_digest.clone(),
                entry.citation_ids_json.clone(),
            );
            if existing != incoming {
                return Err(StoreError::Conflict(format!(
                    "memory usage identity conflict for {}",
                    entry.usage_id
                )));
            }
        }
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct MemorySyncProjection {
    pub(super) record: StoredMemoryRecord,
    pub(super) revision: i64,
}

fn memory_records_equal(left: &StoredMemoryRecord, right: &StoredMemoryRecord) -> bool {
    left.id == right.id
        && left.owner_json == right.owner_json
        && left.namespace_json == right.namespace_json
        && left.scope == right.scope
        && left.kind == right.kind
        && left.content == right.content
        && left.summary == right.summary
        && left.tags_json == right.tags_json
        && left.confidence == right.confidence
        && left.importance == right.importance
        && left.sensitivity == right.sensitivity
        && left.retention_json == right.retention_json
        && left.expires_at == right.expires_at
        && left.supersedes_json == right.supersedes_json
        && left.provenance_json == right.provenance_json
        && left.last_used_at_ms == right.last_used_at_ms
        && left.deleted_at_ms == right.deleted_at_ms
}

pub(super) fn replace_memory_projection(
    transaction: &Connection,
    record: &StoredMemoryRecord,
    revision: i64,
) -> StoreResult<()> {
    validate_memory_record(record)?;
    let previous_owner: Option<String> = transaction
        .query_row(
            "SELECT owner_json FROM memory_records WHERE id=?1",
            params![record.id],
            |row| row.get(0),
        )
        .optional()?;
    if previous_owner
        .as_deref()
        .is_some_and(|owner| owner != record.owner_json)
    {
        return Err(StoreError::Conflict(
            "memory_owner_conflict: sync and replacement cannot change owner".into(),
        ));
    }
    for id in serde_json::from_str::<Vec<String>>(&record.supersedes_json)? {
        let target_owner: Option<String> = transaction
            .query_row(
                "SELECT owner_json FROM memory_records WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        if target_owner
            .as_deref()
            .is_some_and(|owner| owner != record.owner_json)
        {
            return Err(StoreError::Conflict(
                "memory_owner_conflict: supersession crosses owners".into(),
            ));
        }
    }
    let foreign_predecessor: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_records WHERE owner_json<>?1
            AND EXISTS(SELECT 1 FROM json_each(memory_records.supersedes_json) WHERE value=?2))",
        params![record.owner_json, record.id],
        |row| row.get(0),
    )?;
    if foreign_predecessor {
        return Err(StoreError::Conflict(
            "memory_owner_conflict: referenced identity belongs to another owner".into(),
        ));
    }
    transaction.execute(
        "INSERT INTO memory_records(
            id,namespace_json,scope,kind,content,summary,tags_json,confidence,importance,
            sensitivity,retention_json,expires_at,supersedes_json,provenance_json,
            created_at_ms,updated_at_ms,last_used_at_ms,deleted_at_ms,revision,owner_json
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)
         ON CONFLICT(id) DO UPDATE SET owner_json=excluded.owner_json,namespace_json=excluded.namespace_json,
            scope=excluded.scope,kind=excluded.kind,content=excluded.content,
            summary=excluded.summary,tags_json=excluded.tags_json,
            confidence=excluded.confidence,importance=excluded.importance,
            sensitivity=excluded.sensitivity,retention_json=excluded.retention_json,
            expires_at=excluded.expires_at,supersedes_json=excluded.supersedes_json,
            provenance_json=excluded.provenance_json,updated_at_ms=excluded.updated_at_ms,
            last_used_at_ms=excluded.last_used_at_ms,deleted_at_ms=excluded.deleted_at_ms,
            revision=excluded.revision",
        params![
            record.id,
            record.namespace_json,
            record.scope,
            record.kind,
            record.content,
            record.summary,
            record.tags_json,
            record.confidence,
            record.importance,
            record.sensitivity,
            record.retention_json,
            record.expires_at,
            record.supersedes_json,
            record.provenance_json,
            record.created_at_ms,
            record.updated_at_ms,
            record.last_used_at_ms,
            record.deleted_at_ms,
            revision,
            record.owner_json
        ],
    )?;
    crate::memory_index::sync_memory_bm25_on_write(transaction, record, revision)?;
    Ok(())
}

/// Admission shared by ordinary candidates and background maintenance output.
/// Explicit replacement and sync have separate revision-aware contracts.
pub(super) fn validate_memory_candidate_identity(
    connection: &Connection,
    record: &StoredMemoryRecord,
) -> StoreResult<Option<StoredMemoryRecord>> {
    for id in serde_json::from_str::<Vec<String>>(&record.supersedes_json)? {
        let owner: Option<String> = connection
            .query_row(
                "SELECT owner_json FROM memory_records WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        if owner
            .as_deref()
            .is_some_and(|owner| owner != record.owner_json)
        {
            return Err(StoreError::Conflict(
                "memory_owner_conflict: supersession crosses owners".into(),
            ));
        }
    }
    let previous = connection
        .query_row(
            &format!("SELECT {MEMORY_COLUMNS} FROM memory_records WHERE id=?1"),
            params![record.id],
            memory_record_row,
        )
        .optional()?;
    if let Some(previous) = &previous {
        if previous.deleted_at_ms.is_some() {
            return Err(StoreError::Invariant(
                "memory_deleted_conflict: a deleted identity cannot be committed again".into(),
            ));
        }
        if previous.owner_json != record.owner_json
            || previous.namespace_json != record.namespace_json
            || previous.scope != record.scope
        {
            return Err(StoreError::Invariant("memory_scope_conflict: ordinary commits cannot change an existing identity's scope or namespace".into()));
        }
        if previous.updated_at_ms > record.updated_at_ms {
            return Err(StoreError::Invariant(
                "memory_stale_candidate: reload the current memory before updating".into(),
            ));
        }
    }
    Ok(previous)
}

pub(super) fn upsert_memory_projection(
    transaction: &Connection,
    record: &StoredMemoryRecord,
) -> StoreResult<StoredMemoryRecord> {
    let existing = transaction
        .query_row(
            &format!("SELECT {MEMORY_COLUMNS},revision FROM memory_records WHERE id=?1"),
            params![record.id],
            |row| Ok((memory_record_row(row)?, row.get::<_, i64>(19)?)),
        )
        .optional()?;
    if let Some((stored, _)) = existing.as_ref() {
        if stored.owner_json != record.owner_json {
            return Err(StoreError::Conflict(
                "memory_owner_conflict: an identity cannot change owner".into(),
            ));
        }
        if memory_records_equal(stored, record) {
            return Ok(stored.clone());
        }
    }
    let revision = existing
        .as_ref()
        .map_or(1, |(_, revision)| revision.saturating_add(1));
    let mut canonical = record.clone();
    if let Some((stored, _)) = existing.as_ref() {
        canonical.created_at_ms = stored.created_at_ms;
    }
    replace_memory_projection(transaction, &canonical, revision)?;
    let payload = serde_json::to_string(&MemorySyncProjection {
        record: canonical.clone(),
        revision,
    })?;
    enqueue_sync_change(
        transaction,
        "memory",
        &canonical.id,
        revision,
        false,
        &payload,
        canonical.updated_at_ms,
    )?;
    Ok(canonical)
}
