// SPDX-License-Identifier: Apache-2.0
//! Platform-neutral canonical content changes, without persistence or runtime IO.
use crate::{CanonicalStreamHead, CanonicalStreamOperation};
use serde::{Deserialize, Serialize};

/// Bounded original-text facts, without hydrating or reconstructing a prefix.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CanonicalTextState {
    Missing,
    Existing {
        kind: String,
        state: String,
        total_utf8_bytes: u64,
    },
    Stream(CanonicalStreamHead),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MessagePartInput {
    pub id: String,
    pub kind: String,
    pub ordinal: i64,
    pub state: String,
    pub content_text: String,
    pub content_json: Option<String>,
    pub content_digest: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TransitionMessageInput {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub status: String,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    #[serde(default)]
    pub initial_parts: Vec<MessagePartInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PartMutation {
    Stream {
        operation: CanonicalStreamOperation,
        updated_at_ms: i64,
    },
    /// Metadata only. Closing a live stream requires its explicit Close first;
    /// Store enforces that invariant against the committed stream head.
    Metadata {
        part_id: String,
        state: String,
        content_json: Option<String>,
        updated_at_ms: i64,
    },
    Upsert {
        part_id: String,
        kind: String,
        ordinal: i64,
        state: String,
        content_text: String,
        content_json: Option<String>,
        content_digest: Option<String>,
        created_at_ms: i64,
        updated_at_ms: i64,
    },
    AppendText {
        part_id: String,
        expected_content_digest: String,
        text: String,
        updated_at_ms: i64,
    },
    Close {
        part_id: String,
        state: String,
        content_json: Option<String>,
        content_digest: Option<String>,
        updated_at_ms: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplicaMessageDeclaration {
    pub execution_ownership: crate::ConversationExecutionOwnership,
    pub ordinal: i64,
    pub input: TransitionMessageInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplicaOwnedPartMutation {
    pub message_id: String,
    pub run_id: Option<String>,
    pub mutation: PartMutation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplicaAssistantStatus {
    pub message_id: String,
    pub run_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationTransitionReplica {
    pub conversation_id: String,
    pub base_content_revision: i64,
    pub revision: i64,
    pub updated_at_ms: i64,
    pub messages: Vec<ReplicaMessageDeclaration>,
    pub parts: Vec<ReplicaOwnedPartMutation>,
    pub assistant_status: Option<ReplicaAssistantStatus>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::json!({"conversation_id":"c","base_content_revision":1,"revision":2,"updated_at_ms":3,
            "messages":[{"execution_ownership":{"kind":"user"},"ordinal":2,"input":{"id":"m","run_id":"r","role":"assistant","status":"streaming",
                "created_at_ms":3,"completed_at_ms":null,"initial_parts":[]}}],
            "parts":[{"message_id":"m","run_id":"r","mutation":{"type":"append_text","part_id":"p",
                "expected_content_digest":"prefix","text":"next","updated_at_ms":3}}],"assistant_status":null})
    }
    #[test]
    fn source_execution_ownership_is_required() {
        let mut value = fixture();
        value["messages"][0].as_object_mut().unwrap().remove("execution_ownership");
        assert!(serde_json::from_value::<ConversationTransitionReplica>(value).is_err());
    }

    #[test]
    fn canonical_changes_round_trip_without_runtime_fields() {
        let value = fixture();
        let change: ConversationTransitionReplica = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(change).unwrap(), value);
    }
    #[test]
    fn removed_aggregate_replacement_channel_is_rejected() {
        let mut value = fixture();
        value["assistant_replacements"] = serde_json::json!([]);
        assert!(serde_json::from_value::<ConversationTransitionReplica>(value).is_err());
    }

    #[test]
    fn unexpected_runtime_fields_are_rejected_at_every_content_boundary() {
        for path in [
            "",
            "/messages/0",
            "/messages/0/input",
            "/parts/0",
            "/parts/0/mutation",
        ] {
            let mut value = fixture();
            value
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("checkpoint".into(), serde_json::json!({}));
            assert!(
                serde_json::from_value::<ConversationTransitionReplica>(value).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn stream_and_metadata_wire_have_no_implicit_body_or_digest() {
        let owner = crate::CanonicalStreamOwner {
            conversation_id: "c".into(),
            run_id: "r".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: crate::PartBodyField::ContentText,
        };
        let stream = PartMutation::Stream {
            operation: CanonicalStreamOperation::Declare { owner },
            updated_at_ms: 1,
        };
        let metadata = PartMutation::Metadata {
            part_id: "p".into(),
            state: "open".into(),
            content_json: None,
            updated_at_ms: 1,
        };
        for mutation in [stream, metadata] {
            let wire = serde_json::to_value(&mutation).unwrap();
            assert_eq!(
                serde_json::from_value::<PartMutation>(wire.clone()).unwrap(),
                mutation
            );
            for field in ["content_text", "content_digest"] {
                let mut invalid = wire.clone();
                invalid[field] = "hidden body".into();
                assert!(serde_json::from_value::<PartMutation>(invalid).is_err());
            }
            let mut invalid = wire;
            invalid.as_object_mut().unwrap().remove("updated_at_ms");
            assert!(serde_json::from_value::<PartMutation>(invalid).is_err());
        }
    }

    #[test]
    fn text_state_preserves_complete_head_and_requires_existing_facts() {
        let owner = crate::CanonicalStreamOwner {
            conversation_id: "c".into(),
            run_id: "r".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: crate::PartBodyField::ContentText,
        };
        let head =
            crate::apply_canonical_stream(None, &CanonicalStreamOperation::Declare { owner })
                .unwrap();
        for state in [
            CanonicalTextState::Missing,
            CanonicalTextState::Existing {
                kind: "text".into(),
                state: "open".into(),
                total_utf8_bytes: 0,
            },
            CanonicalTextState::Stream(head),
        ] {
            let wire = serde_json::to_value(&state).unwrap();
            assert_eq!(
                serde_json::from_value::<CanonicalTextState>(wire.clone()).unwrap(),
                state
            );
            let mut invalid = wire;
            invalid["content_text"] = "not a metadata query".into();
            assert!(serde_json::from_value::<CanonicalTextState>(invalid).is_err());
        }
        for key in ["kind", "state", "total_utf8_bytes"] {
            let mut wire = serde_json::json!({"type":"existing","value":{"kind":"text","state":"open","total_utf8_bytes":0}});
            wire["value"].as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<CanonicalTextState>(wire).is_err());
        }
    }
}
