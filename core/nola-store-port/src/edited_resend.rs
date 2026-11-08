// SPDX-License-Identifier: Apache-2.0
//! Typed local-only resend records in the caller's existing transaction.
use nola_domain::edited_resend::{MessageSendWitness, PreparedEditedResend};

pub trait EditedResendRepository {
    type Error: std::fmt::Display;
    fn message_send_witness(
        &self,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<Option<MessageSendWitness>, Self::Error>;
    fn prepared_edited_resend_by_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<PreparedEditedResend>, Self::Error>;
    fn prepared_edited_resend(
        &self,
        conversation_id: &str,
    ) -> Result<Option<PreparedEditedResend>, Self::Error>;
    /// Authenticated Host-only reader, never the renderer-facing prepared query.
    fn edited_resend_host_material(
        &self,
        target_conversation_id: &str,
    ) -> Result<Option<EditedResendHostMaterial>, Self::Error>;
    /// Immutable preparation. Only the private Run admission consumer may mark it admitted.
    fn put_prepared_edited_resend(
        &mut self,
        value: &PreparedEditedResend,
    ) -> Result<(), Self::Error>;
}

/// Host-private model credential provenance. This deliberately is not a public
/// prepared DTO and must never pass through a renderer-facing command endpoint.
/// The reference grants no authority to restore the old execution request.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResendModelCredentialReference {
    pub adapter: String,
    pub handle: String,
    pub sha256: String,
}

/// Transport only across the authenticated local Host port; never UI/exports/logs.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditedResendHostMaterial {
    pub prepared: PreparedEditedResend,
    pub model_credential_reference: Option<ResendModelCredentialReference>,
    /// Ownership metadata only. Instruction bodies require the separate
    /// authenticated actor/Host + exact child Run read.
    pub expert_child: bool,
}
