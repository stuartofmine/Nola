// SPDX-License-Identifier: Apache-2.0
//! Typed, bounded query over canonical transcript text, independent of compaction.
use nola_protocol::{ConversationRecallCommand, ConversationRecallHit, ConversationTextRange};
use nola_store_port::TranscriptReadRepository;

impl From<nola_protocol::conversation_recall::CanonicalHistoryCommand> for crate::ApplicationCommand {
    fn from(command: nola_protocol::conversation_recall::CanonicalHistoryCommand) -> Self {
        match command { nola_protocol::conversation_recall::CanonicalHistoryCommand::QueryCanonicalHistory { input } => Self::QueryCanonicalHistory { input } }
    }
}

impl From<ConversationRecallCommand> for crate::ApplicationCommand {
    fn from(command: ConversationRecallCommand) -> Self {
        match command {
            ConversationRecallCommand::ReadConversationTextRange {
                conversation_id,
                message_id,
                part_id,
                offset_bytes,
                limit_bytes,
            } => Self::ReadConversationTextRange {
                conversation_id,
                message_id,
                part_id,
                offset_bytes,
                limit_bytes,
            },
            ConversationRecallCommand::SearchConversationText {
                conversation_id,
                query,
                limit,
            } => Self::SearchConversationText {
                conversation_id,
                query,
                limit,
            },
        }
    }
}

pub(crate) fn read_range<R: TranscriptReadRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
    message_id: &str,
    part_id: &str,
    offset_bytes: u64,
    limit_bytes: usize,
) -> Result<ConversationTextRange, String> {
    let request = ConversationRecallCommand::ReadConversationTextRange {
        conversation_id: conversation_id.into(),
        message_id: message_id.into(),
        part_id: part_id.into(),
        offset_bytes,
        limit_bytes,
    };
    request.validate()?;
    let range = store
        .read_conversation_text_range(
            conversation_id,
            message_id,
            part_id,
            offset_bytes,
            limit_bytes,
        )
        .map_err(|error| error.to_string())?;
    let response = ConversationTextRange {
        conversation_id: range.conversation_id,
        message_id: range.message_id,
        part_id: range.part_id,
        offset_bytes: range.offset_bytes,
        next_offset_bytes: range.next_offset_bytes,
        total_utf8_bytes: range.total_utf8_bytes,
        text: range.text,
        end_of_content: range.end_of_content,
    };
    response.validate_for(&request)?;
    Ok(response)
}

pub(crate) fn search<R: TranscriptReadRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<ConversationRecallHit>, String> {
    ConversationRecallCommand::SearchConversationText {
        conversation_id: conversation_id.into(),
        query: query.into(),
        limit,
    }
    .validate()?;
    store
        .search_transcript_text(conversation_id, query, limit)
        .map_err(|error| error.to_string())
        .map(|hits| {
            hits.into_iter()
                .map(|hit| ConversationRecallHit {
                    message_id: hit.message_id,
                    part_id: hit.part_id,
                    role: hit.role,
                    message_ordinal: hit.message_ordinal,
                    snippet: hit.snippet,
                    start_character: hit.start_character,
                })
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_store_port::{StoredTranscriptMatch, StoredTranscriptRange};
    use std::cell::Cell;

    struct Fixture {
        calls: Cell<usize>,
        wrong_owner: bool,
        unavailable: bool,
    }

    impl TranscriptReadRepository for Fixture {
        type Error = &'static str;

        fn query_canonical_history(&self, _: &nola_domain::canonical_history::CanonicalHistoryQuery)
            -> Result<nola_domain::canonical_history::CanonicalHistoryResponse, Self::Error> {
            Err("history query is outside this single-conversation fixture")
        }

        fn read_conversation_text_range(
            &self,
            conversation_id: &str,
            message_id: &str,
            part_id: &str,
            offset_bytes: u64,
            limit_bytes: usize,
        ) -> Result<StoredTranscriptRange, Self::Error> {
            self.calls.set(self.calls.get() + 1);
            if self.unavailable {
                return Err("repository unavailable");
            }
            assert_eq!((offset_bytes, limit_bytes), (7, 4));
            Ok(StoredTranscriptRange {
                conversation_id: if self.wrong_owner {
                    "other"
                } else {
                    conversation_id
                }
                .into(),
                message_id: message_id.into(),
                part_id: part_id.into(),
                offset_bytes,
                next_offset_bytes: 11,
                total_utf8_bytes: 11,
                text: "tail".into(),
                end_of_content: true,
            })
        }

        fn search_transcript_text(
            &self,
            conversation_id: &str,
            query: &str,
            limit: usize,
        ) -> Result<Vec<StoredTranscriptMatch>, Self::Error> {
            self.calls.set(self.calls.get() + 1);
            assert_eq!((conversation_id, query, limit), ("c", "%_literal", 2));
            Ok(vec![StoredTranscriptMatch {
                message_id: "m".into(),
                part_id: "p".into(),
                role: "user".into(),
                message_ordinal: 3,
                snippet: "%_literal".into(),
                start_character: 7,
            }])
        }
    }

    fn fixture() -> Fixture {
        Fixture {
            calls: Cell::new(0),
            wrong_owner: false,
            unavailable: false,
        }
    }

    #[test]
    fn recall_uses_repository_without_sqlite_and_preserves_literal_query() {
        let store = fixture();
        let port: &dyn TranscriptReadRepository<Error = &'static str> = &store;
        let range = read_range(port, "c", "m", "p", 7, 4).unwrap();
        assert_eq!(range.text, "tail");
        assert!(range.end_of_content);
        let hits = search(port, "c", "%_literal", 2).unwrap();
        assert_eq!(hits[0].snippet, "%_literal");
        assert_eq!(hits[0].start_character, 7);
        assert_eq!(store.calls.get(), 2);
    }

    #[test]
    fn invalid_requests_never_reach_repository() {
        let store = fixture();
        assert!(read_range(&store, "c", "m", "p", 7, 3).is_err());
        assert!(search(&store, "c", "", 2).is_err());
        assert_eq!(store.calls.get(), 0);
    }

    #[test]
    fn repository_errors_and_wrong_owner_do_not_become_successful_reads() {
        let mut store = fixture();
        store.wrong_owner = true;
        assert!(read_range(&store, "c", "m", "p", 7, 4).is_err());
        store.unavailable = true;
        assert_eq!(
            read_range(&store, "c", "m", "p", 7, 4).unwrap_err(),
            "repository unavailable"
        );
    }
}
