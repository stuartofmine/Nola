// SPDX-License-Identifier: Apache-2.0
//! Ordered replica pages bounded by encoded bytes, not merely row count.
use super::*;
use std::io::{self, Write};

// The public protocol omits several Store fields, so bounding the larger Store
// representation also bounds the command result. Leave room in the 16 MiB frame
// for its envelope; never encode a second payload just to measure this bound.
pub const MAX_PENDING_SYNC_JSON_BYTES: usize = 12 * 1024 * 1024;
const MAX_PENDING_SYNC_ITEMS: usize = 512;

struct ByteCounter {
    used: usize,
    maximum: usize,
    exceeded: bool,
}
impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.maximum.saturating_sub(self.used) {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "replica page byte budget exhausted",
            ));
        }
        self.used += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn oversized_entry() -> StoreError {
    StoreError::Invariant(format!(
        "pending sync entry exceeds {MAX_PENDING_SYNC_JSON_BYTES} encoded bytes; chunked content transfer required; entry remains pending",
    ))
}

impl NolaStore {
    pub fn pending_sync_changes(&self, limit: usize) -> StoreResult<Vec<StoredSyncChange>> {
        let mut statement = self.connection.prepare(
            "SELECT local_sequence,operation_id,entity_type,entity_id,revision,deleted,
                CASE WHEN length(CAST(payload_json AS BLOB))<=?2 THEN payload_json ELSE NULL END,
                status,remote_cursor,created_at_ms,resolved_at_ms,
                length(CAST(payload_json AS BLOB))
             FROM sync_outbox WHERE status='pending' ORDER BY local_sequence ASC LIMIT ?1",
        )?;
        let mut rows = statement.query(params![
            limit.clamp(1, MAX_PENDING_SYNC_ITEMS) as i64,
            MAX_PENDING_SYNC_JSON_BYTES as i64
        ])?;
        let mut page = Vec::new();
        let mut used = 2usize; // JSON array delimiters
        while let Some(row) = rows.next()? {
            let raw_bytes: i64 = row.get(11)?;
            let separator = usize::from(!page.is_empty());
            let remaining = MAX_PENDING_SYNC_JSON_BYTES.saturating_sub(used + separator);
            if raw_bytes < 0 || raw_bytes as u64 > remaining as u64 {
                if page.is_empty() {
                    return Err(oversized_entry());
                }
                break;
            }
            let item = sync_change_row(row)?;
            // A remote immutable seal must be acknowledged before its Part
            // reference is offered to any transport, including embedded hosts.
            if item.entity_type == "conversation_append" {
                #[derive(serde::Deserialize)]
                struct Dependencies {
                    #[serde(default)]
                    references: Vec<nola_domain::CanonicalBodyReference>,
                }
                let dependencies: Dependencies = serde_json::from_str(&item.payload_json)?;
                let mut ready = true;
                for reference in dependencies.references {
                    let acknowledged: bool = self.connection.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sync_outbox WHERE entity_type='canonical_text_seal'
                         AND entity_id=?1 AND revision=1 AND status IN ('acknowledged','unchanged') AND payload_json=?2)",
                        params![reference.content_id(),serde_json::to_string(&reference)?], |row| row.get(0))?;
                    ready &= acknowledged;
                }
                if !ready {
                    if page.is_empty() {
                        return Err(StoreError::Conflict("replica content dependency is not acknowledged; append remains pending".into()));
                    }
                    break;
                }
            }
            if item.entity_type == "archive_restore_commit"
                && !archive_restore_sync::dependencies_acknowledged(
                    &self.connection,
                    &item.payload_json,
                )?
            {
                if page.is_empty() {
                    return Err(StoreError::Conflict(
                        "archive staging dependencies are not acknowledged; commit remains pending"
                            .into(),
                    ));
                }
                break;
            }
            if item.entity_type == "conversation_branch_lineage"
                && !crate::part_derivation_sync::dependencies_acknowledged(
                    &self.connection,
                    &item.payload_json,
                )?
            {
                if page.is_empty() {
                    return Err(StoreError::Conflict(
                        "branch Part derivations are not acknowledged; lineage remains pending"
                            .into(),
                    ));
                }
                break;
            }
            let mut counter = ByteCounter {
                used: 0,
                maximum: remaining,
                exceeded: false,
            };
            match serde_json::to_writer(&mut counter, &item) {
                Ok(()) => {}
                Err(_) if counter.exceeded => {
                    if page.is_empty() {
                        return Err(oversized_entry());
                    }
                    break;
                }
                Err(error) => return Err(error.into()),
            }
            used += separator + counter.used;
            page.push(item);
        }
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn enqueue(store: &NolaStore, id: &str, payload: &str) {
        enqueue_sync_change(&store.connection, "record", id, 1, false, payload, 1).unwrap();
    }
    #[test]
    fn pages_keep_queue_order_and_unreturned_records_pending() {
        let root = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(root.path().join("page.db")).unwrap();
        let payload = serde_json::to_string(&"x".repeat(7 * 1024 * 1024)).unwrap();
        enqueue(&store, "first", &payload);
        enqueue(&store, "second", &payload);
        let page = store.pending_sync_changes(100).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].entity_id, "first");
        assert!(serde_json::to_vec(&page).unwrap().len() <= MAX_PENDING_SYNC_JSON_BYTES);
        store
            .resolve_sync_change(&page[0].operation_id, "acknowledged", Some(1), 2)
            .unwrap();
        assert_eq!(
            store.pending_sync_changes(100).unwrap()[0].entity_id,
            "second"
        );
    }
    #[test]
    fn escaping_counts_toward_the_page_budget() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(root.path().join("escaped.db")).unwrap();
        let payload = serde_json::to_string(&"\n".repeat(3 * 1024 * 1024)).unwrap();
        enqueue(&store, "first", &payload);
        enqueue(&store, "second", &payload);
        let page = store.pending_sync_changes(10).unwrap();
        assert_eq!(page.len(), 1);
        assert!(serde_json::to_vec(&page).unwrap().len() <= MAX_PENDING_SYNC_JSON_BYTES);
    }
    #[test]
    fn oversized_head_is_neither_silently_skipped_nor_acknowledged() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(root.path().join("oversized.db")).unwrap();
        let payload = serde_json::to_string(&"x".repeat(MAX_PENDING_SYNC_JSON_BYTES)).unwrap();
        enqueue(&store, "oversized", &payload);
        enqueue(&store, "later", "{}");
        assert!(store
            .pending_sync_changes(10)
            .unwrap_err()
            .to_string()
            .contains("chunked content transfer required"));
        let pending: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sync_outbox WHERE status='pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 2);
    }
    #[test]
    fn caller_count_cannot_overflow_into_an_unlimited_sql_query() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(root.path().join("count.db")).unwrap();
        store.connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        for index in 0..600 {
            enqueue(&store, &format!("row-{index}"), "{}");
        }
        store.connection.execute_batch("COMMIT").unwrap();
        assert_eq!(
            store.pending_sync_changes(usize::MAX).unwrap().len(),
            MAX_PENDING_SYNC_ITEMS
        );
        assert_eq!(store.pending_sync_changes(0).unwrap().len(), 1);
    }
}
