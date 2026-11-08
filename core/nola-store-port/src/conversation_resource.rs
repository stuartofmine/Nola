// SPDX-License-Identifier: Apache-2.0
//! Immutable, conversation-owned binary preview resources.

use serde::{Deserialize, Serialize};

pub const MAX_PDF_RESOURCE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutConversationPdfResource {
    pub conversation_id: String,
    pub resource_id: String,
    pub display_name: String,
    pub mime_type: String,
    pub data_base64: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationResourceDescriptor {
    pub conversation_id: String,
    pub resource_id: String,
    pub display_name: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConversationResourceContent {
    pub descriptor: ConversationResourceDescriptor,
    pub data_base64: String,
}

pub trait ConversationResourceRepository {
    type Error: std::fmt::Display;
    fn put_conversation_pdf_resource(
        &mut self,
        input: &PutConversationPdfResource,
    ) -> Result<Option<ConversationResourceDescriptor>, Self::Error>;
    fn read_conversation_resource(
        &self,
        conversation_id: &str,
        resource_id: &str,
    ) -> Result<Option<ConversationResourceContent>, Self::Error>;
    fn list_conversation_resources(
        &self,
        conversation_id: &str,
    ) -> Result<Option<Vec<ConversationResourceDescriptor>>, Self::Error>;
    fn delete_conversation_resource(
        &mut self,
        conversation_id: &str,
        resource_id: &str,
    ) -> Result<bool, Self::Error>;
}
