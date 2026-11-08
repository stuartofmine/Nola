// SPDX-License-Identifier: Apache-2.0
//! Bounded transcript excerpts. Never materialize the conversation to search it.
use super::*;

pub(crate) fn literal_position(text: &str, query: &str) -> i64 {
    let Some(&first) = query.as_bytes().first() else {
        return 1;
    };
    let bytes = text.as_bytes();
    for offset in memchr::memchr2_iter(
        first.to_ascii_lowercase(),
        first.to_ascii_uppercase(),
        bytes,
    ) {
        if bytes
            .get(offset..offset.saturating_add(query.len()))
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(query.as_bytes()))
        {
            // A valid UTF-8 query starts at an ASCII or leading byte, never
            // a continuation byte, so a match is always a character boundary.
            return text[..offset].chars().count() as i64 + 1;
        }
    }
    0
}

pub use nola_store_port::StoredTranscriptMatch;

impl NolaStore {
    pub fn search_transcript_text(
        &self,
        conversation_id: &str,
        query: &str,
        limit: usize,
    ) -> StoreResult<Vec<StoredTranscriptMatch>> {
        require_id("conversation_id", conversation_id)?;
        if query.trim().is_empty() || query.chars().count() > 500 || !(1..=20).contains(&limit) {
            return Err(StoreError::Invariant(
                "invalid transcript search query or limit".into(),
            ));
        }
        // The index is a rebuildable accelerator; read-only stores or an
        // unavailable index retain canonical literal search functionality.
        let ranked = self.ranked_transcript_candidates(conversation_id, query).unwrap_or_default();
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        self.connection.create_scalar_function(
            "nola_literal_position",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                let text = context
                    .get_raw(0)
                    .as_str()
                    .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                let query = context
                    .get_raw(1)
                    .as_str()
                    .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                Ok(literal_position(text, query))
            },
        )?;
        let mut hits = Vec::new();
        let mut part_ordinals = std::collections::HashMap::new();
        let mut seen = std::collections::HashSet::new();
        let mut terms = crate::transcript_index::terms(query);
        terms.sort_by_key(|term| std::cmp::Reverse(term.len()));
        terms.dedup();
        for (message, part) in ranked {
            if !seen.insert(part.clone()) { continue; }
            let metadata: Option<(String,i64,i64)> = self.connection.query_row(
                "SELECT m.role,m.ordinal,p.ordinal FROM messages m JOIN message_parts p ON p.message_id=m.id
                WHERE m.conversation_id=?1 AND m.id=?2 AND p.id=?3 AND p.state='completed'
                AND COALESCE((SELECT e.visible FROM message_projection_events e
                JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
                WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
                ORDER BY o.resulting_revision DESC LIMIT 1),1)=1",
                params![conversation_id,message,part], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
            let Some((role, ordinal, part_ordinal)) = metadata else { continue; };
            if !crate::canonical_text_disclosure::allows_product_text(&self.connection, conversation_id, &message, &part)? { continue; }
            let reader = crate::canonical_field_reader::CanonicalFieldReader::open(&self.connection,
                conversation_id, &message, &part, nola_domain::PartBodyField::ContentText)?;
            let mut found = reader.find_literal(query)?;
            if found.is_none() {
                for term in &terms {
                    found = reader.find_literal(term)?;
                    if found.is_some() { break; }
                }
            }
            if let Some((position,snippet)) = found {
                part_ordinals.insert(part.clone(), part_ordinal);
                hits.push(StoredTranscriptMatch { message_id:message,part_id:part,role,
                    message_ordinal:ordinal,snippet,start_character:position as i64 });
            }
            if hits.len() == limit { break; }
        }
        if hits.len() < limit {
            for hit in self.search_registered(conversation_id, query, limit)? {
                if !hits.iter().any(|existing| existing.part_id == hit.part_id) {
                    let ordinal: i64 = self.connection.query_row(
                        "SELECT ordinal FROM message_parts WHERE id=?1 AND message_id=?2",
                        params![hit.part_id, hit.message_id], |row| row.get(0))?;
                    part_ordinals.insert(hit.part_id.clone(), ordinal);
                    hits.push(hit);
                }
                if hits.len() == limit { break; }
            }
        }
        // Select by relevance, present the selected evidence chronologically.
        hits.sort_by_key(|hit| (hit.message_ordinal, part_ordinals[&hit.part_id]));
        Ok(hits)
    }

    /// Caller validates the request and registers the borrowed literal UDF.
    /// Tests may instrument that UDF without replacing the production scan.
    fn search_registered(
        &self,
        conversation_id: &str,
        query: &str,
        limit: usize,
    ) -> StoreResult<Vec<StoredTranscriptMatch>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let mut statement = self.connection.prepare(
            "SELECT m.id,p.id,m.role,m.ordinal
             FROM messages m JOIN message_parts p ON p.message_id=m.id
             WHERE m.conversation_id=?1 AND p.kind='text' AND p.state='completed'
               AND COALESCE((SELECT e.visible FROM message_projection_events e
                 JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
                 WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
                 ORDER BY o.resulting_revision DESC LIMIT 1),1)=1
             ORDER BY m.ordinal DESC,p.ordinal DESC",
        )?;
        let mut candidates = statement.query(params![conversation_id])?;
        let mut matches = Vec::new();
        while matches.len() < limit {
            let Some(row) = candidates.next()? else { break };
            let message_id: String = row.get(0)?;
            let part_id: String = row.get(1)?;
            let role: String = row.get(2)?;
            let message_ordinal: i64 = row.get(3)?;
            if !crate::canonical_text_disclosure::allows_product_text(
                &self.connection,
                conversation_id,
                &message_id,
                &part_id,
            )? {
                continue;
            }
            let field = crate::canonical_field_reader::CanonicalFieldReader::open(
                &self.connection,
                conversation_id,
                &message_id,
                &part_id,
                nola_domain::PartBodyField::ContentText,
            )?;
            if let Some((start_character, snippet)) = field.find_literal(query)? {
                matches.push(StoredTranscriptMatch {
                    message_id,
                    part_id,
                    role,
                    message_ordinal,
                    snippet,
                    start_character: i64::try_from(start_character).map_err(|_| {
                        StoreError::Invariant("search character offset overflow".into())
                    })?,
                });
            }
        }
        matches.reverse();
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bm25_recall_indexes_chinese_and_keeps_session_scope() {
        let (_directory, store) = fixture();
        part(&store, "ranked-chinese", "ma", "缓存命中率偏低，需要保留请求前缀。error_code_42");
        let candidates = store.ranked_transcript_candidates("a", "请求 缓存").unwrap();
        assert!(candidates.iter().any(|(_, part)| part == "ranked-chinese"));
        let other = store.ranked_transcript_candidates("b", "error_code_42").unwrap();
        assert!(!other.iter().any(|(_, part)| part == "ranked-chinese"));
        let count: i64 = store.connection.query_row("SELECT COUNT(*) FROM transcript_index_heads WHERE part_id='ranked-chinese'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
        let again = store.ranked_transcript_candidates("a", "error_code_42").unwrap();
        assert!(again.iter().any(|(_, part)| part == "ranked-chinese"));
        store.connection.execute("DELETE FROM message_parts WHERE id='ranked-chinese'", []).unwrap();
        let remaining: i64 = store.connection.query_row("SELECT COUNT(*) FROM transcript_bm25 WHERE part='ranked-chinese'", [], |row| row.get(0)).unwrap();
        assert_eq!(remaining, 0, "deleting original text must also delete derived search data");
    }

    #[test]
    fn query_scans_each_candidate_body_only_once_and_stops_after_latest_hits() {
        use std::sync::{Arc, Mutex};
        let (_directory, store) = fixture();
        for ordinal in 2..=101 {
            let message = format!("m-{ordinal}");
            store
                .connection
                .execute(
                    "INSERT INTO messages VALUES(?1,'a',NULL,'user','completed',?2,1,1)",
                    params![message, ordinal],
                )
                .unwrap();
            part(
                &store,
                &format!("p-{ordinal}"),
                &message,
                &format!("needle-{ordinal}"),
            );
        }
        let counts = Arc::new(Mutex::new(std::collections::HashMap::<String, usize>::new()));
        let observed = counts.clone();
        store
            .connection
            .create_scalar_function(
                "nola_literal_position",
                2,
                rusqlite::functions::FunctionFlags::SQLITE_UTF8
                    | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
                move |context| {
                    let text: String = context.get(0)?;
                    let query: String = context.get(1)?;
                    *observed.lock().unwrap().entry(text.clone()).or_default() += 1;
                    Ok(literal_position(&text, &query))
                },
            )
            .unwrap();
        let matches = store.search_registered("a", "needle", 20).unwrap();
        let ordinals = matches
            .iter()
            .map(|hit| hit.message_ordinal)
            .collect::<Vec<_>>();
        assert_eq!(ordinals, (82..=101).collect::<Vec<_>>());
        let counts = counts.lock().unwrap();
        assert!(
            counts.values().all(|count| *count == 1),
            "duplicate body scans: {counts:?}"
        );
        // SQL may inspect metadata for ordering, but the production loop must
        // never load another body after collecting the requested latest hits.
        assert_eq!(counts.len(), 20, "unexpected candidate scans: {counts:?}");
    }

    #[test]
    fn borrowed_matcher_preserves_literal_character_offsets_and_ascii_folding() {
        for (text, query, expected) in [
            ("前文 Needle", "nEEdLe", 4),
            ("前文预算约束", "预算", 3),
            ("100%_literal", "%_", 4),
            ("Résumé", "résumé", 1),
            ("É", "é", 0),
            ("xxx", "xxxx", 0),
            ("", "x", 0),
            ("é😀tail", "TAIL", 3),
        ] {
            assert_eq!(
                literal_position(text, query),
                expected,
                "{text:?} / {query:?}"
            );
        }
    }

    fn fixture() -> (tempfile::TempDir, NolaStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = NolaStore::open(directory.path().join("recall.db")).unwrap();
        store
            .connection
            .execute_batch(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('a','A',1,1,1),('b','B',1,1,1);
             INSERT INTO messages VALUES('ma','a',NULL,'user','completed',1,1,1),
                 ('mb','b',NULL,'assistant','completed',1,1,1);",
            )
            .unwrap();
        (directory, store)
    }

    fn part(store: &NolaStore, id: &str, message: &str, text: &str) {
        store.connection.execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
             VALUES(?1,?2,'text',1,'completed',?3,1,1)", params![id,message,text],
        ).unwrap();
        crate::canonical_text_disclosure::flush(&store.connection).unwrap();
    }

    #[test]
    fn original_recall_excludes_reasoning_tools_and_open_text() {
        let (_directory, store) = fixture();
        part(&store, "canonical", "ma", "needle original");
        for (index, kind, state) in [
            (2, "reasoning", "completed"),
            (3, "tool_result", "completed"),
            (4, "text", "open"),
            (5, "status", "completed"),
        ] {
            store.connection.execute(
                "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
                 VALUES(?1,'ma',?2,?3,?4,'needle noncanonical',1,1)",
                params![format!("excluded-{index}"), kind, index, state],
            ).unwrap();
        }
        let results = store.search_transcript_text("a", "needle", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].part_id, "canonical");
        assert_eq!(results[0].snippet, "needle original");
    }

    #[test]
    fn ten_million_character_tail_match_returns_bounded_excerpt_after_reopen() {
        let (directory, store) = fixture();
        let needle = "history-needle-123456";
        let mut text = "x".repeat(10_000_000 - needle.chars().count());
        text.push_str(needle);
        assert_eq!(text.chars().count(), 10_000_000);
        part(&store, "pa", "ma", &text);
        drop(text);
        drop(store);
        let store = NolaStore::open(directory.path().join("recall.db")).unwrap();
        let start = std::time::Instant::now();
        let matches = store
            .search_transcript_text("a", "history-needle", 8)
            .unwrap();
        eprintln!(
            "10M transcript search: {:?}, excerpt bytes={}",
            start.elapsed(),
            matches[0].snippet.len()
        );
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].message_id, "ma");
        assert_eq!(matches[0].part_id, "pa");
        assert!(matches[0].snippet.contains("history-needle"));
        assert!(matches[0].snippet.chars().count() <= 1600);
        assert!(matches[0].start_character > 9_000_000);
        let start = std::time::Instant::now();
        assert!(store
            .search_transcript_text("a", "absent-needle", 8)
            .unwrap()
            .is_empty());
        eprintln!("10M transcript no-match search: {:?}", start.elapsed());
    }

    #[test]
    fn literal_unicode_matches_do_not_leak_other_conversations_or_metadata() {
        let (_directory, store) = fixture();
        part(
            &store,
            "pa",
            "ma",
            &format!("{}预算约束 100%_literal", "前".repeat(2000)),
        );
        part(&store, "pb", "mb", "private secret");
        let hits = store.search_transcript_text("a", "预算约束", 8).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].snippet.contains("预算约束"));
        assert!(store
            .search_transcript_text("a", "secret", 8)
            .unwrap()
            .is_empty());
        assert!(store
            .search_transcript_text("a", "completed", 8)
            .unwrap()
            .is_empty());
        assert_eq!(store.search_transcript_text("a", "%_", 8).unwrap().len(), 1);
        assert!(store.search_transcript_text("a", "   ", 8).is_err());
        assert!(store.search_transcript_text("a", "预算", 21).is_err());
    }

    #[test]
    fn many_matches_return_only_latest_bounded_parts_in_chronological_order() {
        let (_directory, store) = fixture();
        store.connection.execute_batch("BEGIN").unwrap();
        for ordinal in 2..=2001 {
            let message = format!("message-{ordinal}");
            store
                .connection
                .execute(
                    "INSERT INTO messages VALUES(?1,'a',NULL,'assistant','completed',?2,1,1)",
                    params![message, ordinal],
                )
                .unwrap();
            part(
                &store,
                &format!("part-{ordinal}"),
                &message,
                &format!("needle {}", "z".repeat(5000)),
            );
        }
        store.connection.execute_batch("COMMIT").unwrap();
        let start = std::time::Instant::now();
        let hits = store.search_transcript_text("a", "NEEDLE", 20).unwrap();
        eprintln!("2000-part transcript search: {:?}", start.elapsed());
        assert_eq!(hits.len(), 20);
        assert_eq!(hits.first().unwrap().message_ordinal, 1982);
        assert_eq!(hits.last().unwrap().message_ordinal, 2001);
        assert!(hits.iter().all(|hit| hit.snippet.len() <= 1600));
        assert!(hits
            .windows(2)
            .all(|pair| pair[0].message_ordinal < pair[1].message_ordinal));
        let one = store.search_transcript_text("a", "needle", 1).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].message_ordinal, 2001);
    }
}
