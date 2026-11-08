// SPDX-License-Identifier: Apache-2.0
//! Atomic operation + physical Attempt ledger commit; no provider execution.
use nola_domain::{
    AuxiliaryInvocationOwner, AuxiliaryModelAttempt, AuxiliaryOperation, AuxiliarySourceObservation,
};
pub trait AuxiliaryInvocationRepository {
    type Error: std::fmt::Display;
    fn conversation_auxiliary_sources(
        &self,
        conversation_id: &str,
        after: Option<&nola_domain::AuxiliarySourceCursor>,
        limit: usize,
    ) -> Result<nola_domain::ConversationAuxiliarySourcePage, Self::Error>;
    fn auxiliary_source_slot(
        &self,
        local_owner: &AuxiliaryInvocationOwner,
        scenario: &nola_domain::AuxiliaryInvocationScenario,
    ) -> Result<Option<AuxiliarySourceObservation>, Self::Error>;
    fn auxiliary_source_observation(
        &self,
        local_owner: &AuxiliaryInvocationOwner,
        source_host_id: &str,
        operation_id: &str,
    ) -> Result<Option<AuxiliarySourceObservation>, Self::Error>;
    fn commit_auxiliary_source_observation(
        &mut self,
        local_owner: &AuxiliaryInvocationOwner,
        observation: &AuxiliarySourceObservation,
    ) -> Result<(), Self::Error>;
    fn auxiliary_operation(&self, id: &str) -> Result<Option<AuxiliaryOperation>, Self::Error>;
    fn auxiliary_attempt(
        &self,
        operation_id: &str,
        id: &str,
    ) -> Result<Option<AuxiliaryModelAttempt>, Self::Error>;
    fn auxiliary_attempt_tail(
        &self,
        operation_id: &str,
    ) -> Result<Option<AuxiliaryModelAttempt>, Self::Error>;
    fn commit_auxiliary_invocation(
        &mut self,
        operation: &AuxiliaryOperation,
        expected_revision: Option<i64>,
        attempt: Option<&AuxiliaryModelAttempt>,
        expected_attempt_revision: Option<i64>,
    ) -> Result<(), Self::Error>;
}
