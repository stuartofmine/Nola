// SPDX-License-Identifier: Apache-2.0
//! Public staged archive intent; canonical references remain Service-owned.
pub use nola_domain::archive_restore::{
    ArchiveConversationProfile, ArchiveMessage, ArchivePartUpload, ArchiveRestoreManifestEntry,
    ArchiveRestoreProgress, ArchiveRestoreResult, BeginArchiveRestore, CommitArchiveRestore,
    StageArchiveManifest,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchiveRestoreCommand {
    ArchiveDirectoryRevision {},
    BeginArchiveRestore { input: BeginArchiveRestore },
    ArchiveRestoreProgress { restore_id: String },
    StageArchiveManifest { input: StageArchiveManifest },
    CommitArchiveRestore { input: CommitArchiveRestore },
}

impl crate::CommandMetadata for ArchiveRestoreCommand {
    fn wire_command_name(&self) -> &'static str {
        match self {
            Self::ArchiveDirectoryRevision { .. } => "archive_directory_revision",
            Self::BeginArchiveRestore { .. } => "begin_archive_restore",
            Self::ArchiveRestoreProgress { .. } => "archive_restore_progress",
            Self::StageArchiveManifest { .. } => "stage_archive_manifest",
            Self::CommitArchiveRestore { .. } => "commit_archive_restore",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandMetadata;

    #[test]
    fn public_restore_commands_round_trip_as_strict_manifest_intent() {
        for command in [
            ArchiveRestoreCommand::ArchiveDirectoryRevision {},
            ArchiveRestoreCommand::BeginArchiveRestore {
                input: BeginArchiveRestore {
                    restore_id: "restore".into(),
                    expected_directory_revision: 7,
                    created_at_ms: 1,
                },
            },
            ArchiveRestoreCommand::ArchiveRestoreProgress {
                restore_id: "restore".into(),
            },
            ArchiveRestoreCommand::StageArchiveManifest {
                input: StageArchiveManifest {
                    restore_id: "restore".into(),
                    sequence: 0,
                    entry: ArchiveRestoreManifestEntry::Profile {
                        profile: ArchiveConversationProfile {
                            id: "c".into(),
                            title: "Original".into(),
                            settings_json: None,
                            created_at_ms: 1,
                            updated_at_ms: 2,
                        },
                    },
                },
            },
            ArchiveRestoreCommand::CommitArchiveRestore {
                input: CommitArchiveRestore {
                    restore_id: "restore".into(),
                    expected_directory_revision: 7,
                    expected_entry_count: 2,
                    manifest_digest: "immutable".into(),
                    committed_at_ms: 3,
                },
            },
        ] {
            let mut wire = serde_json::to_value(&command).unwrap();
            assert_eq!(wire["type"], command.wire_command_name());
            assert_eq!(
                serde_json::from_value::<ArchiveRestoreCommand>(wire.clone()).unwrap(),
                command
            );
            wire["path"] = "/tmp/not-an-authorization".into();
            assert!(serde_json::from_value::<ArchiveRestoreCommand>(wire).is_err());
        }
    }

    #[test]
    fn client_cannot_stage_canonical_references_in_place_of_owner_bound_uploads() {
        let command = ArchiveRestoreCommand::StageArchiveManifest {
            input: StageArchiveManifest {
                restore_id: "restore".into(),
                sequence: 1,
                entry: ArchiveRestoreManifestEntry::Message {
                    conversation_id: "c".into(),
                    message: ArchiveMessage {
                        id: "m".into(),
                        run_id: None,
                        role: "user".into(),
                        status: "completed".into(),
                        ordinal: 1,
                        created_at_ms: 1,
                        completed_at_ms: Some(2),
                        parts: Vec::new(),
                    },
                    uploads: Vec::new(),
                },
            },
        };
        let mut wire = serde_json::to_value(command).unwrap();
        wire["input"]["entry"]["references"] = serde_json::json!([]);
        assert!(serde_json::from_value::<ArchiveRestoreCommand>(wire.clone()).is_err());
        wire["input"]["entry"]
            .as_object_mut()
            .unwrap()
            .remove("uploads");
        assert!(serde_json::from_value::<ArchiveRestoreCommand>(wire).is_err());
    }
}
