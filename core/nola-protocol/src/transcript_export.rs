// SPDX-License-Identifier: Apache-2.0
//! Archive export wire commands. Bodies are fetched through bounded byte ranges.
pub use nola_domain::transcript_export::*;
pub use nola_domain::tool_source::{ToolSourceClaim, ToolSourceExportRange};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExportCommand {
    ReadConversationToolSources {
        owner: CanonicalPartOwner,
        expected_conversation_revision: i64,
    },
    ReadConversationToolSourceRange {
        owner: CanonicalPartOwner,
        source_item_id: String,
        expected_conversation_revision: i64,
        offset_bytes: u64,
        limit_bytes: usize,
    },
    ReadConversationPartDerivation {
        owner: CanonicalPartOwner,
        expected_conversation_revision: i64,
    },
    LoadConversationExportPage {
        conversation_id: String,
        after_ordinal: Option<i64>,
        expected_revision: Option<i64>,
    },
    ReadConversationExportRange {
        conversation_id: String,
        message_id: String,
        part_id: String,
        field: ExportBodyField,
        expected_revision: i64,
        offset_bytes: u64,
        limit_bytes: usize,
    },
}

impl crate::CommandMetadata for ExportCommand {
    fn wire_command_name(&self) -> &'static str {
        match self {
            Self::ReadConversationToolSources { .. } => "read_conversation_tool_sources",
            Self::ReadConversationToolSourceRange { .. } => "read_conversation_tool_source_range",
            Self::ReadConversationPartDerivation { .. } => "read_conversation_part_derivation",
            Self::LoadConversationExportPage { .. } => "load_conversation_export_page",
            Self::ReadConversationExportRange { .. } => "read_conversation_export_range",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandMetadata;
    #[test]
    fn export_commands_round_trip_with_strict_fields_and_no_paths() {
        for command in [
            ExportCommand::ReadConversationToolSources {
                owner: CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
                expected_conversation_revision: 7,
            },
            ExportCommand::ReadConversationToolSourceRange {
                owner: CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
                source_item_id: "original-item".into(),
                expected_conversation_revision: 7,
                offset_bytes: 0,
                limit_bytes: 32768,
            },
            ExportCommand::ReadConversationPartDerivation {
                owner: CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
                expected_conversation_revision: 7,
            },
            ExportCommand::LoadConversationExportPage {
                conversation_id: "c".into(),
                after_ordinal: Some(5),
                expected_revision: Some(7),
            },
            ExportCommand::ReadConversationExportRange {
                conversation_id: "c".into(),
                message_id: "m".into(),
                part_id: "p".into(),
                field: ExportBodyField::ContentJson,
                expected_revision: 7,
                offset_bytes: 32768,
                limit_bytes: 32768,
            },
        ] {
            let mut wire = serde_json::to_value(&command).unwrap();
            assert_eq!(wire["type"], command.wire_command_name());
            assert_eq!(
                serde_json::from_value::<ExportCommand>(wire.clone()).unwrap(),
                command
            );
            wire["path"] = "/tmp/unauthorized".into();
            assert!(serde_json::from_value::<ExportCommand>(wire).is_err());
        }
        assert!(
            serde_json::from_str::<ExportBodyField>("\"content_text; SELECT secret\"").is_err()
        );
    }
}
