// SPDX-License-Identifier: Apache-2.0
//! Private aggregate, not generic product-setting access or a renderer proof producer.
use nola_domain::expert_message::{ExpertMessageManifest, PreparedExpertResend};
pub trait ExpertMessageRepository: crate::model::ModelResolutionFactsRepository {
    type Error: std::fmt::Display;
    fn expert_message_manifest(
        &self,
        conversation: &str,
        message: &str,
    ) -> Result<Option<ExpertMessageManifest>, Self::Error>;
    fn expert_message_by_submission(
        &self,
        conversation: &str,
        submission: &str,
    ) -> Result<Option<ExpertMessageManifest>, Self::Error>;
    /// Exact private child index; never a latest-reservation or history scan.
    fn expert_message_for_child(
        &self,
        context: &str,
        run: &str,
    ) -> Result<Option<ExpertMessageManifest>, Self::Error>;
    /// Core Application owns the typed resource/provenance schema. Persistence
    /// stores private JSON without depending on Engine authorization types.
    fn expert_template_capture_fact(
        &self,
        conversation: &str,
        submission: &str,
    ) -> Result<Option<String>, Self::Error>;
    fn put_expert_template_capture_fact(
        &mut self,
        conversation: &str,
        submission: &str,
        fact_json: &str,
        captured_at_ms: i64,
    ) -> Result<(), Self::Error>;
    fn expert_context_reservation(
        &self,
        context: &str,
    ) -> Result<Option<nola_domain::expert_message::ExpertContextReservation>, Self::Error>;
    fn put_expert_message_manifest(
        &mut self,
        value: &ExpertMessageManifest,
    ) -> Result<(), Self::Error>;
    fn prepared_expert_resend(
        &self,
        target: &str,
    ) -> Result<Option<PreparedExpertResend>, Self::Error>;
    fn prepared_expert_resend_by_operation(
        &self,
        operation: &str,
    ) -> Result<Option<PreparedExpertResend>, Self::Error>;
    fn expert_resend_child_instructions(
        &self,
        target: &str,
    ) -> Result<Option<Vec<nola_domain::expert_message::ExpertPromptDefinition>>, Self::Error>;
    fn put_prepared_expert_resend(
        &mut self,
        value: &PreparedExpertResend,
    ) -> Result<(), Self::Error>;
}
