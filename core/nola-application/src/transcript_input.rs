// SPDX-License-Identifier: Apache-2.0
//! Application validation and private persistence conversion for protocol inputs.
use nola_protocol::TranscriptMessageInput;

pub(crate) trait FinalizedTranscriptInput: Sized {
    fn into_store(self) -> nola_store_port::transcript_write::MessageInput;
}

impl FinalizedTranscriptInput for TranscriptMessageInput {
    fn into_store(self) -> nola_store_port::transcript_write::MessageInput {
        nola_store_port::transcript_write::MessageInput {
            id: self.id,
            run_id: self.run_id,
            role: self.role,
            status: self.status,
            ordinal: self.ordinal,
            created_at_ms: self.created_at_ms,
            completed_at_ms: self.completed_at_ms,
            parts: self
                .parts
                .into_iter()
                .map(|part| nola_domain::MessagePartInput {
                    id: part.id,
                    kind: part.kind,
                    ordinal: part.ordinal,
                    state: part.state,
                    content_text: part.content_text,
                    content_json: part.content_json,
                    content_digest: part.content_digest,
                    created_at_ms: part.created_at_ms,
                    updated_at_ms: part.updated_at_ms,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_protocol::TranscriptPartInput;
    fn input() -> TranscriptMessageInput {
        TranscriptMessageInput {
            id: "message".into(),
            run_id: None,
            role: "user".into(),
            status: "completed".into(),
            ordinal: 1,
            created_at_ms: 1,
            completed_at_ms: Some(2),
            parts: vec![TranscriptPartInput {
                id: "part".into(),
                kind: "text".into(),
                ordinal: 1,
                state: "completed".into(),
                content_text: "canonical text".repeat(1000),
                content_json: Some("{}".into()),
                content_digest: None,
                created_at_ms: 1,
                updated_at_ms: 2,
            }],
        }
    }
    #[test]
    fn persistence_conversion_moves_body_and_preserves_all_fields() {
        let input = input();
        let encoded = serde_json::to_value(&input).unwrap();
        let body_address = input.parts[0].content_text.as_ptr();
        let stored = input.into_store();
        assert_eq!(body_address, stored.parts[0].content_text.as_ptr());
        assert_eq!(encoded, serde_json::to_value(stored).unwrap());
    }
    #[test]
    fn client_cannot_smuggle_unrecognized_message_or_part_fields() {
        let mut message = serde_json::to_value(input()).unwrap();
        message["storage_path"] = "arbitrary.sqlite".into();
        assert!(serde_json::from_value::<TranscriptMessageInput>(message).is_err());
        let mut message = serde_json::to_value(input()).unwrap();
        message["parts"][0]["canonical_content_id"] = "unpublished".into();
        assert!(serde_json::from_value::<TranscriptMessageInput>(message).is_err());
    }
}
