// SPDX-License-Identifier: Apache-2.0
//! Pure metadata admission. Application owns base fetching and atomic persistence.
use nola_domain::ConversationMetadataReplica;

pub struct ConversationMetadataState<'a> {
    pub conversation_id: &'a str,
    pub content_revision: Option<u64>,
    pub metadata: Option<&'a ConversationMetadataReplica>,
    pub tombstone_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationMetadataDecision {
    Apply,
    Replay,
    Stale,
    Conflict,
    MissingContentBase { required_revision: u64 },
    Deleted,
    WrongConversation,
    InvalidState,
}

pub fn evaluate_conversation_metadata(
    state: &ConversationMetadataState<'_>,
    incoming: &ConversationMetadataReplica,
) -> ConversationMetadataDecision {
    use ConversationMetadataDecision::*;
    if incoming.conversation_id() != state.conversation_id {
        return WrongConversation;
    }
    if state.content_revision == Some(0)
        || state.tombstone_revision == Some(0)
        || state
            .metadata
            .is_some_and(|metadata| metadata.conversation_id() != state.conversation_id)
    {
        return InvalidState;
    }
    // Conversation IDs are not reused. Resurrection needs a new canonical
    // creation identity, not an arbitrarily larger metadata revision.
    if state.tombstone_revision.is_some() {
        return Deleted;
    }
    if let Some(previous) = state.metadata {
        if incoming.metadata_revision() < previous.metadata_revision() {
            return Stale;
        }
        if incoming.metadata_revision() == previous.metadata_revision() && incoming != previous {
            return Conflict;
        }
    }
    if !state
        .content_revision
        .is_some_and(|revision| revision >= incoming.base_content_revision())
    {
        return MissingContentBase {
            required_revision: incoming.base_content_revision(),
        };
    }
    if state.metadata == Some(incoming) {
        Replay
    } else {
        Apply
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata(revision: u64, base: u64, title: &str) -> ConversationMetadataReplica {
        ConversationMetadataReplica::new("c".into(), revision, base, title.into(), None, 1).unwrap()
    }
    fn state() -> ConversationMetadataState<'static> {
        ConversationMetadataState {
            conversation_id: "c",
            content_revision: Some(2),
            metadata: None,
            tombstone_revision: None,
        }
    }
    #[test]
    fn metadata_waits_for_content_instead_of_accepting_empty_history() {
        let input = metadata(3, 2, "Title");
        for base in [None, Some(1)] {
            let mut state = state();
            state.content_revision = base;
            assert_eq!(
                evaluate_conversation_metadata(&state, &input),
                ConversationMetadataDecision::MissingContentBase {
                    required_revision: 2
                }
            );
        }
        assert_eq!(
            evaluate_conversation_metadata(&state(), &input),
            ConversationMetadataDecision::Apply
        );
    }
    #[test]
    fn equal_revision_requires_identical_metadata_and_available_content() {
        let previous = metadata(3, 2, "Title");
        let mut state = state();
        state.metadata = Some(&previous);
        assert_eq!(
            evaluate_conversation_metadata(&state, &previous),
            ConversationMetadataDecision::Replay
        );
        assert_eq!(
            evaluate_conversation_metadata(&state, &metadata(3, 2, "Other")),
            ConversationMetadataDecision::Conflict
        );
        state.content_revision = None;
        assert_eq!(
            evaluate_conversation_metadata(&state, &previous),
            ConversationMetadataDecision::MissingContentBase {
                required_revision: 2
            }
        );
    }
    #[test]
    fn delayed_metadata_cannot_regress_current_settings() {
        let previous = metadata(4, 2, "New");
        let mut state = state();
        state.metadata = Some(&previous);
        assert_eq!(
            evaluate_conversation_metadata(&state, &metadata(3, 2, "Old")),
            ConversationMetadataDecision::Stale
        );
    }
    #[test]
    fn metadata_cannot_resurrect_a_tombstoned_conversation() {
        let mut state = state();
        state.tombstone_revision = Some(4);
        for revision in [3, 4, 5, u64::MAX] {
            assert_eq!(
                evaluate_conversation_metadata(&state, &metadata(revision, 2, "Late")),
                ConversationMetadataDecision::Deleted
            );
        }
    }
    #[test]
    fn ownership_and_invalid_local_state_are_explicit() {
        let input = metadata(3, 2, "Title");
        let mut wrong = state();
        wrong.conversation_id = "other";
        assert_eq!(
            evaluate_conversation_metadata(&wrong, &input),
            ConversationMetadataDecision::WrongConversation
        );
        let mut invalid = state();
        invalid.content_revision = Some(0);
        assert_eq!(
            evaluate_conversation_metadata(&invalid, &input),
            ConversationMetadataDecision::InvalidState
        );
    }
}
