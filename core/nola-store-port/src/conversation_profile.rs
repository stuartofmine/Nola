// SPDX-License-Identifier: Apache-2.0
//! Conversation identity and profile write contracts without SQLite dependencies.
use serde::{Deserialize, Serialize};

/// Metadata has no execution semantics. Both Application and persistence enforce
/// this allowlist so alternate repository consumers cannot reopen a business write.
pub fn validate_metadata_fields(value: &serde_json::Value) -> Result<(), &'static str> {
    let fields = value.as_object().filter(|fields| !fields.is_empty())
        .ok_or("metadata patch must be a nonempty object")?;
    for (key, value) in fields {
        let valid = match key.as_str() {
            "isPinned" | "isArchived" => value.is_boolean(),
            "clientNote" => value.is_null() || value.is_string(),
            _ => false,
        };
        if !valid { return Err("unsupported metadata field or value; use its owning command"); }
    }
    Ok(())
}

/// Shared native knowledge keys. Omission preserves a field; an empty string
/// clears it. Timestamps are owned by the committed transaction, not the client.
pub use nola_domain::configuration_patch::knowledge_patch_fields;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreateConversationInput {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub settings_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateConversationResult {
    pub created: bool,
    /// Present only for this transaction's new owner, never a read of an
    /// existing owner's latest configuration.
    pub initial_configuration_revision: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PatchConversationConfigurationInput {
    pub conversation_id: String,
    pub expected_revision: i64,
    pub patch_json: nola_domain::configuration_patch::ConversationConfigurationPatch,
    pub updated_at_ms: i64,
}

/// Sparse root-field merge. Null removes a field; omission preserves it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PatchConversationMetadataInput {
    pub conversation_id: String,
    pub expected_revision: i64,
    pub patch_json: String,
    pub updated_at_ms: i64,
}

pub trait ConversationProfileRepository {
    type Error: std::fmt::Display;
    fn patch_conversation_metadata(
        &mut self,
        input: &PatchConversationMetadataInput,
    ) -> Result<Option<i64>, Self::Error>;
    fn patch_conversation_configuration(
        &mut self,
        input: &PatchConversationConfigurationInput,
    ) -> Result<nola_domain::conversation_configuration::ConfigurationCommitResult, Self::Error>;
    fn create_conversation(
        &mut self,
        input: &CreateConversationInput,
    ) -> Result<CreateConversationResult, Self::Error>;
    fn rename_conversation(
        &mut self,
        id: &str,
        title: &str,
        title_source: Option<nola_domain::conversation_configuration::ConversationTitleSource>,
        updated_at_ms: i64,
    ) -> Result<bool, Self::Error>;
}

/// Composition port for canonical conversation write use cases. These traits
/// share the same aggregate and must join the caller-owned Unit of Work, while
/// projection/feed reads remain outside this object.
pub trait ConversationWriteRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: ConversationProfileRepository<Error = Self::Error>
        + crate::canonical_branch::CanonicalBranchRepository<Error = Self::Error>
        + crate::conversation_deletion::ConversationDeletionRepository<Error = Self::Error>
        + crate::transcript_write::TranscriptEditRepository<Error = Self::Error>
    where
        Self: 'a;

    fn conversation_write_repository(&mut self) -> Self::Repository<'_>;
}
