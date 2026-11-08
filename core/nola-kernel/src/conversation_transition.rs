// SPDX-License-Identifier: Apache-2.0
//! Pure admission for canonical content replication. No repository or IO access.
use nola_domain::ConversationTransitionReplica;

pub struct ConversationContentState<'a> {
    pub conversation_id: &'a str,
    pub content_revision: Option<i64>,
    pub tombstone_revision: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationContentDecision {
    Apply,
    Stale,
    Deleted,
    WrongConversation,
    InvalidState,
    InvalidEnvelope,
    MissingContentBase { required_revision: i64 },
    Conflict,
}

pub fn evaluate_conversation_transition(
    state: &ConversationContentState<'_>,
    incoming: &ConversationTransitionReplica,
    envelope_revision: i64,
    deleted: bool,
) -> ConversationContentDecision {
    use ConversationContentDecision::*;
    if incoming.conversation_id != state.conversation_id {
        return WrongConversation;
    }
    if deleted
        || incoming.revision != envelope_revision
        || incoming.base_content_revision <= 0
        || incoming.revision <= incoming.base_content_revision
    {
        return InvalidEnvelope;
    }
    if state.content_revision.is_some_and(|revision| revision <= 0)
        || state
            .tombstone_revision
            .is_some_and(|revision| revision <= 0)
        || (state.content_revision.is_some() && state.tombstone_revision.is_some())
    {
        return InvalidState;
    }
    if state.tombstone_revision.is_some() {
        return Deleted;
    }
    match state.content_revision {
        Some(revision) if revision > incoming.revision => Stale,
        Some(revision) if revision == incoming.base_content_revision => Apply,
        None => MissingContentBase {
            required_revision: incoming.base_content_revision,
        },
        Some(revision) if revision < incoming.base_content_revision => MissingContentBase {
            required_revision: incoming.base_content_revision,
        },
        // Equal resulting revisions require operation-level deduplication by
        // Application. Revision equality alone never authorizes an overwrite.
        Some(_) => Conflict,
    }
}

pub struct ReplicaMessageOwner<'a> {
    pub conversation_id: &'a str,
    pub role: &'a str,
    pub run_id: Option<&'a str>,
}

pub fn accepts_replica_assistant_owner(
    conversation: &str,
    run: Option<&str>,
    stored: Option<&ReplicaMessageOwner<'_>>,
) -> bool {
    stored.is_some_and(|owner| {
        owner.conversation_id == conversation && owner.role == "assistant" && owner.run_id == run
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> ConversationTransitionReplica {
        ConversationTransitionReplica {
            conversation_id: "c".into(),
            base_content_revision: 2,
            revision: 4,
            updated_at_ms: 1,
            messages: Vec::new(),
            parts: Vec::new(),
            assistant_status: None,
        }
    }
    fn state(revision: Option<i64>) -> ConversationContentState<'static> {
        ConversationContentState {
            conversation_id: "c",
            content_revision: revision,
            tombstone_revision: None,
        }
    }

    #[test]
    fn only_an_exact_content_base_can_apply() {
        for base in [None, Some(1)] {
            assert_eq!(
                evaluate_conversation_transition(&state(base), &input(), 4, false),
                ConversationContentDecision::MissingContentBase {
                    required_revision: 2
                }
            );
        }
        assert_eq!(
            evaluate_conversation_transition(&state(Some(2)), &input(), 4, false),
            ConversationContentDecision::Apply
        );
        for base in [3, 4] {
            assert_eq!(
                evaluate_conversation_transition(&state(Some(base)), &input(), 4, false),
                ConversationContentDecision::Conflict
            );
        }
        assert_eq!(
            evaluate_conversation_transition(&state(Some(5)), &input(), 4, false),
            ConversationContentDecision::Stale
        );
    }
    #[test]
    fn malformed_envelopes_and_local_state_are_not_replays() {
        assert_eq!(
            evaluate_conversation_transition(&state(Some(2)), &input(), 5, false),
            ConversationContentDecision::InvalidEnvelope
        );
        assert_eq!(
            evaluate_conversation_transition(&state(Some(2)), &input(), 4, true),
            ConversationContentDecision::InvalidEnvelope
        );
        for base in [0, -1] {
            assert_eq!(
                evaluate_conversation_transition(&state(Some(base)), &input(), 4, false),
                ConversationContentDecision::InvalidState
            );
        }
        let mut invalid = input();
        invalid.base_content_revision = 4;
        assert_eq!(
            evaluate_conversation_transition(&state(Some(2)), &invalid, 4, false),
            ConversationContentDecision::InvalidEnvelope
        );
    }
    #[test]
    fn tombstones_and_conversation_scope_win_over_newer_input() {
        let mut stored = state(None);
        stored.tombstone_revision = Some(1);
        assert_eq!(
            evaluate_conversation_transition(&stored, &input(), 4, false),
            ConversationContentDecision::Deleted
        );
        stored.conversation_id = "other";
        assert_eq!(
            evaluate_conversation_transition(&stored, &input(), 4, false),
            ConversationContentDecision::WrongConversation
        );
        stored.conversation_id = "c";
        stored.content_revision = Some(2);
        assert_eq!(
            evaluate_conversation_transition(&stored, &input(), 4, false),
            ConversationContentDecision::InvalidState
        );
    }
    #[test]
    fn matching_run_does_not_allow_a_foreign_conversation_or_user_message() {
        let mut owner = ReplicaMessageOwner {
            conversation_id: "c",
            role: "assistant",
            run_id: Some("r"),
        };
        assert!(accepts_replica_assistant_owner(
            "c",
            Some("r"),
            Some(&owner)
        ));
        assert!(!accepts_replica_assistant_owner(
            "c",
            Some("other"),
            Some(&owner)
        ));
        assert!(!accepts_replica_assistant_owner("c", Some("r"), None));
        owner.conversation_id = "other";
        assert!(!accepts_replica_assistant_owner(
            "c",
            Some("r"),
            Some(&owner)
        ));
        owner.conversation_id = "c";
        owner.role = "user";
        assert!(!accepts_replica_assistant_owner(
            "c",
            Some("r"),
            Some(&owner)
        ));
    }
}
