// SPDX-License-Identifier: Apache-2.0
//! SQLite memory repository adapter. SQL pattern syntax and BM25 fusion stay here.
use crate::{NolaStore, StoreError};
use nola_store_port::memory::{
    MemoryReadRepository, MemoryRepository, StoredMemoryRecord, StoredMemoryUsageStats,
};
use std::collections::{BTreeMap, BTreeSet};

fn escaped_like_pattern(value: &str) -> String {
    format!(
        "%{}%",
        value
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

struct RankedMemoryCandidate {
    record: StoredMemoryRecord,
    exact_match: bool,
    bm25_score: f64,
}

impl MemoryReadRepository for NolaStore {
    type Error = StoreError;
    fn memory_record(&self, id: &str) -> Result<Option<StoredMemoryRecord>, Self::Error> {
        NolaStore::memory_record(self, id)
    }
    fn memory_record_with_revision(
        &self,
        id: &str,
    ) -> Result<Option<(StoredMemoryRecord, i64)>, Self::Error> {
        NolaStore::memory_record_with_revision(self, id)
    }
    fn memory_records_by_ids(
        &self,
        ids: &[String],
    ) -> Result<Vec<StoredMemoryRecord>, Self::Error> {
        NolaStore::memory_records_by_ids(self, ids)
    }
    fn list_memory_records(&self, limit: usize) -> Result<Vec<StoredMemoryRecord>, Self::Error> {
        NolaStore::list_memory_records(self, limit)
    }
    fn memory_usage_stats(
        &self,
        memory_ids: &[String],
    ) -> Result<Vec<StoredMemoryUsageStats>, Self::Error> {
        NolaStore::memory_usage_stats(self, memory_ids)
    }
    fn recall_memory_records(
        &self,
        owner_json: &str,
        namespace_json: &str,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        now_iso: &str,
        limit: usize,
    ) -> Result<Vec<StoredMemoryRecord>, Self::Error> {
        let normalized_query = query.map(str::trim).filter(|value| !value.is_empty());
        let pattern = normalized_query.map(escaped_like_pattern);
        let exact_records = self.search_memory_records(
            owner_json,
            namespace_json,
            scope,
            pattern.as_deref(),
            kind,
            now_iso,
            limit,
        )?;
        let maximum_candidates = limit.clamp(1, 50).saturating_mul(4).min(200);
        let bm25_candidates = match normalized_query {
            Some(query_text) => self.ranked_memory_candidates(
                owner_json,
                namespace_json,
                scope,
                query_text,
                kind,
                now_iso,
                maximum_candidates,
            )?,
            None => Vec::new(),
        };

        let mut ranked = BTreeMap::<String, RankedMemoryCandidate>::new();
        for record in exact_records {
            ranked.insert(
                record.id.clone(),
                RankedMemoryCandidate {
                    record,
                    exact_match: true,
                    bm25_score: 0.0,
                },
            );
        }
        for (record, raw_score) in bm25_candidates {
            let bm25_score = if raw_score.is_finite() {
                raw_score
            } else {
                0.0
            };
            if let Some(existing) = ranked.get_mut(&record.id) {
                existing.bm25_score = existing.bm25_score.min(bm25_score);
            } else {
                ranked.insert(
                    record.id.clone(),
                    RankedMemoryCandidate {
                        record,
                        exact_match: false,
                        bm25_score,
                    },
                );
            }
        }

        let mut ranked = ranked.into_values().collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .exact_match
                .cmp(&left.exact_match)
                .then_with(|| left.bm25_score.total_cmp(&right.bm25_score))
                .then_with(|| right.record.importance.total_cmp(&left.record.importance))
                .then_with(|| right.record.confidence.total_cmp(&left.record.confidence))
                .then_with(|| right.record.updated_at_ms.cmp(&left.record.updated_at_ms))
                .then_with(|| right.record.id.cmp(&left.record.id))
        });

        let mut records = ranked
            .into_iter()
            .take(maximum_candidates)
            .map(|candidate| candidate.record)
            .collect::<Vec<_>>();
        // Literal and BM25 retrieval can find an older statement without finding
        // its differently-worded correction. Complete only the successor chain of
        // this bounded result so Core can apply canonical supersession policy.
        let mut seen = records
            .iter()
            .map(|record| record.id.clone())
            .collect::<BTreeSet<_>>();
        let mut cursor = 0;
        while cursor < records.len() && records.len() < maximum_candidates {
            let remaining = maximum_candidates - records.len();
            let successors = self.memory_records_superseding(
                owner_json,
                namespace_json,
                scope,
                &records[cursor].id,
                now_iso,
                remaining,
            )?;
            for successor in successors {
                if seen.insert(successor.id.clone()) {
                    records.push(successor);
                }
            }
            cursor += 1;
        }
        Ok(records)
    }
}

impl MemoryRepository for NolaStore {
    fn has_write_unit_of_work(&self) -> bool {
        NolaStore::has_write_unit_of_work(self)
    }
    fn upsert_memory_record(
        &mut self,
        record: &StoredMemoryRecord,
    ) -> Result<StoredMemoryRecord, Self::Error> {
        NolaStore::upsert_memory_record(self, record)
    }
    fn save_memory_record_deduplicated(
        &mut self,
        record: &StoredMemoryRecord,
    ) -> Result<StoredMemoryRecord, Self::Error> {
        NolaStore::save_memory_record_deduplicated(self, record)
    }
    fn delete_memory_record(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error> {
        NolaStore::delete_memory_record(self, id, deleted_at_ms)
    }
}
