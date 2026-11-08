// SPDX-License-Identifier: Apache-2.0
//! Rebuildable, session-scoped BM25 candidate index. Canonical readers remain
//! authoritative for visibility, ownership and returned original text.
use super::*;

pub(super) fn terms(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut previous = None;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            word.push(ch.to_ascii_lowercase());
            previous = None;
        } else {
            if !word.is_empty() { result.push(std::mem::take(&mut word)); }
            if !ch.is_ascii() && ch.is_alphanumeric() {
                result.push(ch.to_string());
                if let Some(first) = previous { result.push(format!("{first}{ch}")); }
                previous = Some(ch);
            } else { previous = None; }
        }
    }
    if !word.is_empty() { result.push(word); }
    result
}

impl NolaStore {
    pub(super) fn ranked_transcript_candidates(&self, conversation: &str, query: &str)
        -> StoreResult<Vec<(String, String)>> {
        let transaction = if self.connection.is_autocommit() {
            Some(self.connection.unchecked_transaction()?)
        } else { None };
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS transcript_index_heads(
            part_id TEXT PRIMARY KEY, stamp TEXT NOT NULL);
            CREATE VIRTUAL TABLE IF NOT EXISTS transcript_bm25 USING fts5(
                conversation UNINDEXED, message UNINDEXED, part UNINDEXED, terms,
                tokenize='unicode61');
            CREATE TRIGGER IF NOT EXISTS transcript_index_delete AFTER DELETE ON message_parts
            WHEN OLD.kind='text' BEGIN
                DELETE FROM transcript_bm25 WHERE part=OLD.id;
                DELETE FROM transcript_index_heads WHERE part_id=OLD.id;
            END;
            CREATE TRIGGER IF NOT EXISTS transcript_index_invalidate
            AFTER UPDATE OF content_text,content_json,content_digest,state,updated_at_ms ON message_parts
            WHEN OLD.kind='text' AND OLD.state='completed' BEGIN
                DELETE FROM transcript_bm25 WHERE part=OLD.id;
                DELETE FROM transcript_index_heads WHERE part_id=OLD.id;
            END;")?;
        let mut pending = self.connection.prepare("SELECT m.id,p.id,
            CAST(p.updated_at_ms AS TEXT)||':'||COALESCE(p.content_digest,'') AS stamp
            FROM messages m JOIN message_parts p ON p.message_id=m.id
            WHERE m.conversation_id=?1 AND p.kind='text' AND p.state='completed'
            AND NOT EXISTS(SELECT 1 FROM transcript_index_heads h WHERE h.part_id=p.id
                AND h.stamp=CAST(p.updated_at_ms AS TEXT)||':'||COALESCE(p.content_digest,''))
            ORDER BY m.ordinal DESC,p.ordinal DESC LIMIT 128")?;
        let rows = pending.query_map([conversation], |row| Ok((row.get::<_,String>(0)?,
            row.get::<_,String>(1)?, row.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
        drop(pending);
        let mut indexed_bytes = 0u64;
        for (message, part, stamp) in rows {
            if !crate::canonical_text_disclosure::allows_product_text(&self.connection, conversation, &message, &part)? { continue; }
            let reader = crate::canonical_field_reader::CanonicalFieldReader::open(
                &self.connection, conversation, &message, &part, nola_domain::PartBodyField::ContentText)?;
            // Very large bodies remain searchable through the canonical scan;
            // maintenance must not block an interactive query indefinitely.
            if reader.total_utf8_bytes().unwrap_or(0) > 1024 * 1024 { continue; }
            indexed_bytes = indexed_bytes.saturating_add(reader.total_utf8_bytes().unwrap_or(0));
            if indexed_bytes > 4 * 1024 * 1024 { break; }
            self.connection.execute("DELETE FROM transcript_bm25 WHERE part=?1", [&part])?;
            let mut offset = 0;
            loop {
                let range = reader.read_range(offset, 16384)?;
                let indexed = terms(&range.text).join(" ");
                self.connection.execute("INSERT INTO transcript_bm25(conversation,message,part,terms)
                    VALUES(?1,?2,?3,?4)", params![conversation,message,part,indexed])?;
                if range.end_of_content { break; }
                if range.next_offset_bytes <= offset { return Err(StoreError::Invariant("transcript index made no progress".into())); }
                offset = range.next_offset_bytes;
            }
            self.connection.execute("INSERT INTO transcript_index_heads(part_id,stamp) VALUES(?1,?2)
                ON CONFLICT(part_id) DO UPDATE SET stamp=excluded.stamp", params![part,stamp])?;
        }
        let incomplete: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM messages m
            JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id=?1
            AND p.kind='text' AND p.state='completed' AND NOT EXISTS(
                SELECT 1 FROM transcript_index_heads h WHERE h.part_id=p.id
                AND h.stamp=CAST(p.updated_at_ms AS TEXT)||':'||COALESCE(p.content_digest,'')))",
            [conversation], |row| row.get(0))?;
        if incomplete {
            if let Some(transaction) = transaction { transaction.commit()?; }
            return Ok(Vec::new());
        }
        let mut query_terms = terms(query);
        query_terms.sort(); query_terms.dedup(); query_terms.truncate(32);
        if query_terms.is_empty() {
            if let Some(transaction) = transaction { transaction.commit()?; }
            return Ok(Vec::new());
        }
        let expression = query_terms.iter().map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>().join(" OR ");
        let mut statement = self.connection.prepare("SELECT message,part FROM transcript_bm25
            JOIN messages m ON m.id=transcript_bm25.message
            JOIN message_parts p ON p.id=transcript_bm25.part AND p.message_id=m.id
            WHERE transcript_bm25 MATCH ?1 AND conversation=?2
            ORDER BY bm25(transcript_bm25),m.ordinal DESC,p.ordinal DESC LIMIT 80")?;
        let result = statement.query_map(params![expression,conversation], |row| Ok((row.get(0)?,row.get(1)?)))?
            .collect::<Result<Vec<_>,_>>()?;
        drop(statement);
        if let Some(transaction) = transaction { transaction.commit()?; }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn index_terms_cover_chinese_and_identifiers_without_query_operators() {
        let tokens = terms("缓存命中 missing_field /src/file.rs");
        assert!(tokens.contains(&"缓存".into()));
        assert!(tokens.contains(&"missing_field".into()));
        assert!(!terms("OR * \"").contains(&"*".into()));
    }
}
