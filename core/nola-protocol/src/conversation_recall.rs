// SPDX-License-Identifier: Apache-2.0
//! Bounded canonical transcript queries, not provider context or UI caches.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonicalHistoryCommand {
    QueryCanonicalHistory { input: nola_domain::canonical_history::CanonicalHistoryQuery },
}
impl crate::CommandMetadata for CanonicalHistoryCommand {
    fn wire_command_name(&self) -> &'static str { "query_canonical_history" }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationRecallCommand {
    ReadConversationTextRange {
        conversation_id: String,
        message_id: String,
        part_id: String,
        offset_bytes: u64,
        limit_bytes: usize,
    },
    SearchConversationText {
        conversation_id: String,
        query: String,
        limit: usize,
    },
}
impl ConversationRecallCommand {
    pub fn validate(&self) -> Result<(), String> {
        let identity = |id: &str| {
            if id.is_empty()
                || id.trim() != id
                || id.len() > 128
                || id.chars().any(char::is_control)
            {
                Err("invalid canonical transcript identity".to_string())
            } else {
                Ok(())
            }
        };
        match self {
            Self::ReadConversationTextRange {
                conversation_id,
                message_id,
                part_id,
                offset_bytes,
                limit_bytes,
            } => {
                for id in [conversation_id, message_id, part_id] {
                    identity(id)?;
                }
                if !(4..=32_768).contains(limit_bytes) || *offset_bytes >= i64::MAX as u64 {
                    return Err("invalid canonical transcript byte range".into());
                }
            }
            Self::SearchConversationText {
                conversation_id,
                query,
                limit,
            } => {
                identity(conversation_id)?;
                if query.trim().is_empty()
                    || query.chars().count() > 500
                    || !(1..=20).contains(limit)
                {
                    return Err("invalid canonical transcript search bounds".into());
                }
            }
        }
        Ok(())
    }
}
impl crate::CommandMetadata for ConversationRecallCommand {
    fn wire_command_name(&self) -> &'static str {
        match self {
            Self::ReadConversationTextRange { .. } => "read_conversation_text_range",
            Self::SearchConversationText { .. } => "search_conversation_text",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationTextRange {
    pub conversation_id: String,
    pub message_id: String,
    pub part_id: String,
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub total_utf8_bytes: u64,
    pub text: String,
    pub end_of_content: bool,
}
impl ConversationTextRange {
    pub fn validate_for(&self, request: &ConversationRecallCommand) -> Result<(), String> {
        request.validate()?;
        let ConversationRecallCommand::ReadConversationTextRange {
            conversation_id,
            message_id,
            part_id,
            offset_bytes,
            limit_bytes,
        } = request
        else {
            return Err("text range requires a range request".into());
        };
        if &self.conversation_id != conversation_id
            || &self.message_id != message_id
            || &self.part_id != part_id
            || self.offset_bytes != *offset_bytes
        {
            return Err("canonical text response owner or offset mismatch".into());
        }
        if self.text.len() > *limit_bytes
            || self.offset_bytes.checked_add(self.text.len() as u64) != Some(self.next_offset_bytes)
            || self.next_offset_bytes > self.total_utf8_bytes
            || self.end_of_content != (self.next_offset_bytes == self.total_utf8_bytes)
            || (self.text.is_empty() && !self.end_of_content)
        {
            return Err("canonical text response has invalid bounds or makes no progress".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationRecallHit {
    pub message_id: String,
    pub part_id: String,
    pub role: String,
    pub message_ordinal: i64,
    pub snippet: String,
    pub start_character: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandMetadata;
    #[test]
    fn response_validation_rejects_cross_owner_and_nonprogressing_ranges() {
        let request = ConversationRecallCommand::ReadConversationTextRange {
            conversation_id: "c".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            offset_bytes: 0,
            limit_bytes: 4,
        };
        let valid = ConversationTextRange {
            conversation_id: "c".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            offset_bytes: 0,
            next_offset_bytes: 3,
            total_utf8_bytes: 10,
            text: "abc".into(),
            end_of_content: false,
        };
        valid.validate_for(&request).unwrap();
        for bad in [
            ConversationTextRange {
                conversation_id: "other".into(),
                ..valid.clone()
            },
            ConversationTextRange {
                message_id: "other".into(),
                ..valid.clone()
            },
            ConversationTextRange {
                part_id: "other".into(),
                ..valid.clone()
            },
            ConversationTextRange {
                offset_bytes: 1,
                ..valid.clone()
            },
            ConversationTextRange {
                next_offset_bytes: 4,
                ..valid.clone()
            },
            ConversationTextRange {
                total_utf8_bytes: 2,
                ..valid.clone()
            },
            ConversationTextRange {
                text: "abcde".into(),
                next_offset_bytes: 5,
                ..valid.clone()
            },
            ConversationTextRange {
                end_of_content: true,
                ..valid.clone()
            },
            ConversationTextRange {
                text: String::new(),
                next_offset_bytes: 0,
                ..valid.clone()
            },
        ] {
            assert!(bad.validate_for(&request).is_err());
        }
        let end = ConversationTextRange {
            text: String::new(),
            next_offset_bytes: 0,
            total_utf8_bytes: 0,
            end_of_content: true,
            ..valid
        };
        end.validate_for(&request).unwrap();
    }
    #[test]
    fn query_commands_have_stable_wire_names_and_round_trip() {
        for command in [
            ConversationRecallCommand::ReadConversationTextRange {
                conversation_id: "c".into(),
                message_id: "m".into(),
                part_id: "p".into(),
                offset_bytes: 9_999_991,
                limit_bytes: 32_768,
            },
            ConversationRecallCommand::SearchConversationText {
                conversation_id: "c".into(),
                query: "original evidence".into(),
                limit: 10,
            },
        ] {
            let mut value = serde_json::to_value(&command).unwrap();
            assert_eq!(value["type"], command.wire_command_name());
            assert_eq!(
                serde_json::from_value::<ConversationRecallCommand>(value.clone()).unwrap(),
                command
            );
            value["readOnly"] = true.into();
            assert!(serde_json::from_value::<ConversationRecallCommand>(value).is_err());
        }
    }
    #[test]
    fn small_range_retains_original_length_and_owner_identity() {
        let range = ConversationTextRange {
            conversation_id: "c".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            offset_bytes: 9_999_991,
            next_offset_bytes: 10_000_000,
            total_utf8_bytes: 10_000_000,
            text: "TAIL-DONE".into(),
            end_of_content: true,
        };
        let bytes = serde_json::to_vec(&range).unwrap();
        assert!(bytes.len() < 1024);
        assert_eq!(
            serde_json::from_slice::<ConversationTextRange>(&bytes).unwrap(),
            range
        );
    }
}
