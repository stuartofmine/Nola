// SPDX-License-Identifier: Apache-2.0
//! Canonical identity of a Core observation, independent of reducer state.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventCursor {
    pub sequence: u64,
}

/// The source fields retained alongside the original event payload in Store.
/// A session-level event legitimately has no Turn; consumers verify ownership
/// against the event kind and their enclosing committed transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreEventSource {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub cursor: EventCursor,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreEventEnvelope<E> {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub cursor: EventCursor,
    pub revision: u64,
    pub event: E,
}

impl<E> CoreEventEnvelope<E> {
    pub fn source(&self) -> CoreEventSource {
        CoreEventSource {
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            cursor: self.cursor,
            revision: self.revision,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_preserves_exact_identity_without_copying_event_body() {
        let event = CoreEventEnvelope {
            session_id: "session".into(),
            turn_id: None,
            cursor: EventCursor { sequence: 8 },
            revision: 4,
            event: serde_json::json!({"body":"original"}),
        };
        let source = serde_json::to_value(event.source()).unwrap();
        assert_eq!(
            source,
            serde_json::json!({"session_id":"session","turn_id":null,"cursor":{"sequence":8},"revision":4})
        );
        assert!(
            serde_json::from_value::<CoreEventSource>(serde_json::to_value(event).unwrap())
                .is_err()
        );
    }
    #[test]
    fn identity_parser_rejects_unknown_fields_and_alternate_cursor_shapes() {
        let source = serde_json::json!({"session_id":"s","turn_id":"t","cursor":{"sequence":1},"revision":1});
        for (key, value) in [
            ("cursor", serde_json::json!(1)),
            ("cursor", serde_json::json!({"sequence":1,"extra":2})),
            ("extra", serde_json::json!(1)),
        ] {
            let mut invalid = source.clone();
            invalid[key] = value;
            assert!(serde_json::from_value::<CoreEventSource>(invalid).is_err());
        }
        let mut event = source;
        event["event"] = serde_json::json!({});
        event["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<CoreEventEnvelope<serde_json::Value>>(event).is_err());
    }
}
