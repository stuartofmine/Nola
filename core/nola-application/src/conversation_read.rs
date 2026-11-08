// SPDX-License-Identifier: Apache-2.0
//! Stable conversation read models owned by the Application layer.
//!
//! Store rows are persistence details. Hosts consume these projections so a
//! schema migration cannot silently become a cross-platform protocol change.

use nola_store_port::conversation_history::{StoredConversation, StoredMessage, StoredPart};
use nola_store_port::conversation_query::ConversationSummary as StoreConversationSummary;
use nola_store_port::run_effect::{StoredActiveRun, StoredCheckpoint};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPart {
    pub id: String,
    pub kind: String,
    pub ordinal: i64,
    pub state: String,
    pub content_text: String,
    pub content_json: Option<String>,
    /// Read projection metadata: the original JSON exists but was not hydrated.
    /// This is not a persisted Part field or authority to read its bytes.
    pub content_json_deferred: bool,
    pub content_digest: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationMessage {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub status: String,
    pub ordinal: i64,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub parts: Vec<ConversationPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub configuration_revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub settings_json: Option<String>,
    pub messages: Vec<ConversationMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub message_count: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub project_root: Option<String>,
    pub first_user_message: Option<String>,
    pub last_user_message_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationTailSnapshot {
    pub conversation: Conversation,
    pub total_message_count: usize,
    pub total_user_turns: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ActiveRun {
    pub run_id: String,
    pub kind: String,
    pub assistant_message_id: Option<String>,
    pub next_event_sequence: i64,
    pub invocation_config_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationCheckpoint {
    pub conversation_id: String,
    pub revision: i64,
    pub event_cursor: i64,
    pub checkpoint_json: String,
    pub updated_at_ms: i64,
}

impl From<StoredPart> for ConversationPart {
    fn from(value: StoredPart) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            ordinal: value.ordinal,
            state: value.state,
            content_text: value.content_text,
            content_json: value.content_json,
            content_json_deferred: value.content_json_deferred,
            content_digest: value.content_digest,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
        }
    }
}

impl From<StoredMessage> for ConversationMessage {
    fn from(value: StoredMessage) -> Self {
        Self {
            id: value.id,
            run_id: value.run_id,
            role: value.role,
            status: value.status,
            ordinal: value.ordinal,
            created_at_ms: value.created_at_ms,
            completed_at_ms: value.completed_at_ms,
            parts: value.parts.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<StoredConversation> for Conversation {
    fn from(value: StoredConversation) -> Self {
        Self {
            id: value.id,
            title: value.title,
            revision: value.revision,
            configuration_revision: value.configuration_revision,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            settings_json: value.settings_json,
            messages: value.messages.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<StoreConversationSummary> for ConversationSummary {
    fn from(value: StoreConversationSummary) -> Self {
        Self {
            id: value.id,
            title: value.title,
            revision: value.revision,
            message_count: value.message_count,
            created_at_ms: value.created_at_ms,
            updated_at_ms: value.updated_at_ms,
            project_root: value.project_root,
            first_user_message: value.first_user_message,
            last_user_message_at_ms: value.last_user_message_at_ms,
        }
    }
}

impl TryFrom<nola_store_port::conversation_query::StoredConversationTailSnapshot>
    for ConversationTailSnapshot
{
    type Error = String;

    fn try_from(
        value: nola_store_port::conversation_query::StoredConversationTailSnapshot,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            conversation: value.conversation.into(),
            total_message_count: usize::try_from(value.total_message_count)
                .map_err(|_| "conversation tail message count is negative".to_string())?,
            total_user_turns: usize::try_from(value.total_user_turns)
                .map_err(|_| "conversation tail user turn count is negative".to_string())?,
        })
    }
}

impl From<StoredActiveRun> for ActiveRun {
    fn from(value: StoredActiveRun) -> Self {
        Self {
            run_id: value.run_id,
            kind: value.kind,
            assistant_message_id: value.assistant_message_id,
            next_event_sequence: value.next_event_sequence,
            invocation_config_digest: value.invocation_config_digest,
        }
    }
}

impl From<StoredCheckpoint> for ConversationCheckpoint {
    fn from(value: StoredCheckpoint) -> Self {
        Self {
            conversation_id: value.conversation_id,
            revision: value.revision,
            event_cursor: value.event_cursor,
            checkpoint_json: value.checkpoint_json,
            updated_at_ms: value.updated_at_ms,
        }
    }
}

#[cfg(test)]
mod json_deferred_tests {
    use super::*;

    fn part(deferred: bool) -> StoredPart {
        StoredPart {
            id: "p".into(),
            kind: "text".into(),
            ordinal: 0,
            state: "completed".into(),
            content_text: String::new(),
            content_json: None,
            content_json_deferred: deferred,
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn application_mapping_preserves_explicit_absent_vs_deferred_json() {
        for deferred in [false, true] {
            let stored = part(deferred);
            let stored_wire = serde_json::to_value(&stored).unwrap();
            assert_eq!(stored_wire["content_json_deferred"], deferred);
            let application = ConversationPart::from(stored);
            assert_eq!(application.content_json_deferred, deferred);
            assert_eq!(application.content_json, None);
            let wire = serde_json::to_value(&application).unwrap();
            assert_eq!(wire["content_json_deferred"], deferred);
            assert_eq!(
                serde_json::from_value::<ConversationPart>(wire).unwrap(),
                application
            );
        }
    }

    #[test]
    fn omitted_deferred_fact_is_not_silently_decoded_as_absence() {
        let mut wire = serde_json::to_value(part(false)).unwrap();
        wire.as_object_mut()
            .unwrap()
            .remove("content_json_deferred");
        assert!(serde_json::from_value::<StoredPart>(wire.clone()).is_err());
        assert!(serde_json::from_value::<ConversationPart>(wire).is_err());
    }
}
