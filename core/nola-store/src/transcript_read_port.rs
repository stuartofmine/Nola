// SPDX-License-Identifier: Apache-2.0
//! SQLite adapter for the bounded transcript read port.
use crate::{NolaStore, StoreError};
use nola_store_port::{StoredTranscriptMatch, StoredTranscriptRange, TranscriptReadRepository};

impl TranscriptReadRepository for NolaStore {
    type Error = StoreError;

    fn query_canonical_history(
        &self,
        input: &nola_domain::canonical_history::CanonicalHistoryQuery,
    ) -> Result<nola_domain::canonical_history::CanonicalHistoryResponse, Self::Error> {
        use nola_domain::canonical_history::*;
        use rusqlite::{params, OptionalExtension};
        input.validate().map_err(StoreError::Invariant)?;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let requester = input.requester();
        let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            [requester], |row| row.get(0))?;
        if !exists { return Err(StoreError::Conflict("canonical history requester has no local conversation authority".into())); }
        match input {
            CanonicalHistoryQuery::Read { conversation_id, message_id, part_id, expected_conversation_revision, offset_bytes, limit_bytes, .. } => {
                let (title, revision): (String, i64) = self.connection.query_row(
                    "SELECT title,revision FROM conversations WHERE id=?1", [conversation_id],
                    |row| Ok((row.get(0)?,row.get(1)?))).optional()?
                    .ok_or_else(|| StoreError::Conflict("canonical history source is unavailable in this local user Store".into()))?;
                if expected_conversation_revision.is_some_and(|expected| expected != revision) {
                    return Err(StoreError::Conflict("canonical history source changed after selection; search or select its current revision".into()));
                }
                let range = self.read_conversation_text_range(conversation_id, message_id, part_id, *offset_bytes, *limit_bytes)?;
                Ok(CanonicalHistoryResponse::Read { requester_conversation_id: requester.into(),
                    source: CanonicalHistorySource { conversation_id: range.conversation_id, conversation_title: title,
                        conversation_revision: revision, message_id: range.message_id, part_id: range.part_id },
                    offset_bytes: range.offset_bytes, next_offset_bytes: range.next_offset_bytes,
                    total_utf8_bytes: range.total_utf8_bytes, text: range.text, end_of_content: range.end_of_content })
            }
            CanonicalHistoryQuery::Search { scope, query, limit, after_conversation_id, .. } => {
                let selected = match scope {
                    CanonicalHistoryScope::CurrentConversation => Some(vec![requester.to_owned()]),
                    CanonicalHistoryScope::LocalUserHistory => None,
                    CanonicalHistoryScope::Conversations { conversation_ids } => Some(conversation_ids.clone()),
                };
                if let Some(ids) = &selected {
                    for id in ids {
                        let exists: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
                            [id], |row|row.get(0))?;
                        if !exists { return Err(StoreError::Conflict("selected history conversation is outside the available local user Store".into())); }
                    }
                }
                let selected_json = selected.as_ref().map(serde_json::to_string).transpose()?;
                let mut statement = self.connection.prepare("SELECT id,title,revision FROM conversations
                    WHERE (?1 IS NULL OR id>?1) AND (?2 IS NULL OR id IN (SELECT value FROM json_each(?2)))
                    ORDER BY id LIMIT 17")?;
                let candidates = statement.query_map(params![after_conversation_id,selected_json], |row|
                    Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?)))?
                    .collect::<Result<Vec<_>,_>>()?;
                drop(statement);
                let mut hits = Vec::new();
                let mut searched = Vec::new();
                let mut results_limited = false;
                for (id, title, revision) in candidates.iter().take(16) {
                    let matches = self.search_transcript_text(id, query, 20)?;
                    results_limited |= matches.len() == 20 || matches.len() > limit.saturating_sub(hits.len());
                    searched.push(id.clone());
                    for hit in matches.into_iter().take(limit.saturating_sub(hits.len())) {
                        hits.push(CanonicalHistoryHit { source: CanonicalHistorySource {
                            conversation_id: id.clone(), conversation_title: title.clone(), conversation_revision: *revision,
                            message_id: hit.message_id, part_id: hit.part_id },
                            role: hit.role, message_ordinal: hit.message_ordinal, snippet: hit.snippet, start_character: hit.start_character });
                    }
                    if hits.len() == *limit { results_limited = true; break; }
                }
                let next_conversation_cursor = if searched.len() < candidates.len() { searched.last().cloned() } else { None };
                Ok(CanonicalHistoryResponse::Search { requester_conversation_id: requester.into(), hits,
                    searched_conversation_ids: searched, next_conversation_cursor, results_limited })
            }
        }
    }

    fn read_conversation_text_range(
        &self,
        conversation_id: &str,
        message_id: &str,
        part_id: &str,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> Result<StoredTranscriptRange, Self::Error> {
        NolaStore::read_conversation_text_range(
            self,
            conversation_id,
            message_id,
            part_id,
            offset_bytes,
            limit_bytes,
        )
    }

    fn search_transcript_text(
        &self,
        conversation_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StoredTranscriptMatch>, Self::Error> {
        NolaStore::search_transcript_text(self, conversation_id, query, limit)
    }
}
