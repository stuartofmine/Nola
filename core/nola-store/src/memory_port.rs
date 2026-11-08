// SPDX-License-Identifier: Apache-2.0
//! SQLite memory repository adapter. SQL pattern syntax stays here.
use crate::{NolaStore, StoreError};
use nola_store_port::memory::{
    MemoryReadRepository, MemoryRepository, StoredMemoryRecord, StoredMemoryUsageStats,
};
use std::collections::{BTreeMap, BTreeSet};

const MAX_RECALL_QUERY_TERMS: usize = 12;

fn escaped_like_pattern(value: &str) -> String {
    format!(
        "%{}%",
        value
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

/// Produce a small, language-neutral lexical fallback when an exact phrase
/// does not occur in a stored memory. Whitespace-delimited text uses words;
/// unsegmented Unicode text uses overlapping character pairs so CJK and other
/// scripts do not require a language-specific dictionary.
fn recall_query_terms(query: &str) -> Vec<String> {
    let mut terms = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2)
        .map(|term| term.to_lowercase())
        .collect::<Vec<_>>();
    if terms.len() == 1 && terms[0].chars().count() >= 4 && !terms[0].is_ascii() {
        let characters = terms[0].chars().collect::<Vec<_>>();
        terms = characters
            .windows(2)
            .map(|pair| pair.iter().collect::<String>().to_lowercase())
            .collect();
    }
    let mut seen = BTreeSet::new();
    terms.retain(|term| seen.insert(term.clone()));
    // Prefer more discriminative terms when a long natural-language question
    // exceeds the bounded query budget. Equal-length terms retain source order.
    terms.sort_by_key(|term| std::cmp::Reverse(term.chars().count()));
    terms.truncate(MAX_RECALL_QUERY_TERMS);
    terms
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
        let terms = normalized_query.map(recall_query_terms).unwrap_or_default();
        let exact_score = terms.len().saturating_add(1);
        let mut ranked = BTreeMap::<String, (StoredMemoryRecord, usize, bool)>::new();
        for record in exact_records {
            ranked.insert(record.id.clone(), (record, exact_score, true));
        }
        for term in &terms {
            let pattern = escaped_like_pattern(term);
            for record in self.search_memory_records(
                owner_json,
                namespace_json,
                scope,
                Some(&pattern),
                kind,
                now_iso,
                limit.clamp(1, 50),
            )? {
                let entry = ranked
                    .entry(record.id.clone())
                    .or_insert((record, 0, false));
                entry.1 = entry.1.saturating_add(1);
            }
        }
        let mut ranked = ranked.into_values().collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .2
                .cmp(&left.2)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| right.0.importance.total_cmp(&left.0.importance))
                .then_with(|| right.0.confidence.total_cmp(&left.0.confidence))
                .then_with(|| right.0.updated_at_ms.cmp(&left.0.updated_at_ms))
                .then_with(|| right.0.id.cmp(&left.0.id))
        });
        let mut records = ranked
            .into_iter()
            .take(maximum_candidates)
            .map(|(record, _, _)| record)
            .collect::<Vec<_>>();
        // Literal retrieval can find an older statement without finding its
        // differently-worded correction. Complete only the successor chain of
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
