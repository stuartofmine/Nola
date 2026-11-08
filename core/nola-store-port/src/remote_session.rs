// SPDX-License-Identifier: Apache-2.0
//! Remote Host persistence and authoritative session read contracts.
use crate::run_effect::StoredCheckpoint;
use nola_domain::remote_submission::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalRunTerminalStatus { Sealed }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalRunTerminal {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub status: CanonicalRunTerminalStatus,
    pub result: nola_domain::run_result::RunResult,
    pub result_digest: nola_domain::assistant_journal::AssistantDigest,
    pub completed_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdmittedReplicaTurnInput {
    pub source_host_id: String,
    pub conversation_id: String,
    pub conversation_title: String,
    pub user_message_id: String,
    pub user_part_id: String,
    pub user_part_kind: String,
    pub user_text: String,
    #[serde(default)]
    pub user_content_json: Option<String>,
    pub assistant_message_id: String,
    pub assistant_part_id: String,
    pub run_id: String,
    pub context_checkpoint_id: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthoritativeSessionRecord {
    Local(StoredLocalSessionRecovery),
}
/// Authority, replay frontier, and restore evidence captured in one read snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLocalSessionRecovery {
    pub recovery: crate::run_effect::StoredRecoveryCheckpoint,
    pub invocation: Option<crate::run_effect::StoredRunInvocationConfig>,
    pub delegated_invocations: Vec<nola_domain::DelegatedInvocationRecord>,
}
impl From<StoredCheckpoint> for StoredLocalSessionRecovery {
    fn from(checkpoint: StoredCheckpoint) -> Self {
        Self {
            recovery: crate::run_effect::StoredRecoveryCheckpoint {
                checkpoint,
                replay_base: None,
                replay_inputs: Vec::new(),
                frozen_model_requests: Vec::new(),
            },
            invocation: None,
            delegated_invocations: Vec::new(),
        }
    }
}
/// Choose authority and read its record within one snapshot; reuse caller UoW.
pub trait SessionProjectionRepository {
    type Error: std::fmt::Display;
    fn load_authoritative_session(
        &self,
        conversation_id: &str,
    ) -> Result<Option<AuthoritativeSessionRecord>, Self::Error>;
}
/// Mutations keep projection/mirror, grants and caller receipt in one UoW.
pub trait RemoteSessionRepository {
    type Error: std::fmt::Display;
    fn abandon_prepared_remote_submission(
        &mut self,
        identity: &RemoteSubmissionIdentity,
    ) -> Result<RemoteSubmissionIntentView, Self::Error>;
    fn adopt_observed_remote_run(
        &mut self,
        receipt: &SourceSubmissionReceipt,
    ) -> Result<(), Self::Error>;
    /// Identity admitted on this replica, from its originating intent or
    /// explicitly adopted source receipt. Never reads source-owned receipts.
    fn admitted_remote_run_identity(
        &self, run_id: &str,
    ) -> Result<Option<RemoteSubmissionIdentity>, Self::Error>;
    fn observed_source_submission_receipt(
        &self,
        source_host_id: &str,
        client_id: &str,
        conversation_id: &str,
        run_id: &str,
        assistant_message_id: &str,
    ) -> Result<Option<SourceSubmissionReceipt>, Self::Error>;
    fn canonical_run_terminal(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<CanonicalRunTerminal>, Self::Error>;
    fn prepare_remote_submission(
        &mut self,
        input: &PrepareRemoteSubmissionInput,
    ) -> Result<RemoteSubmissionIntentView, Self::Error>;
    fn bind_remote_submission_request(
        &mut self,
        identity: &RemoteSubmissionIdentity,
        request: &serde_json::Value,
    ) -> Result<RemoteSubmissionIntentView, Self::Error>;
    fn remote_submission_intent(
        &self,
        conversation_id: &str,
    ) -> Result<Option<RemoteSubmissionIntentView>, Self::Error>;
    fn remote_submission_request(
        &self,
        identity: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> Result<serde_json::Value, Self::Error>;
    /// Exact bound local intent, including the phase of an accepted source
    /// receipt. Unlike the composer query, terminal intents remain observable.
    fn exact_bound_remote_submission(
        &self,
        identity: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> Result<RemoteSubmissionIntentView, Self::Error>;
    fn register_source_submission(
        &mut self,
        identity: &RemoteSubmissionIdentity,
        request: &serde_json::Value,
    ) -> Result<RegisteredSourceSubmission, Self::Error>;
    fn source_submission_receipt(
        &self,
        identity: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> Result<Option<SourceSubmissionReceipt>, Self::Error>;
    fn settle_source_submission(
        &mut self,
        identity: &RemoteSubmissionIdentity,
        digest: &str,
        rejection: Option<&str>,
    ) -> Result<SourceSubmissionReceipt, Self::Error>;
    fn accept_remote_submission_receipt(
        &mut self,
        receipt: &SourceSubmissionReceipt,
    ) -> Result<RemoteSubmissionIntentView, Self::Error>;
    fn assistant_message_id_for_run(
        &self,
        run_id: &str,
        conversation_id: &str,
    ) -> Result<Option<String>, Self::Error>;
}

/// Narrow read capability for canonical task/journal availability. Publication
/// does not acquire remote submission or replica mutation authority.
pub trait CanonicalRunTerminalRepository {
    type Error: std::fmt::Display;
    fn read_canonical_run_terminal(&self, conversation_id: &str, run_id: &str)
        -> Result<Option<CanonicalRunTerminal>, Self::Error>;
}
impl<T: RemoteSessionRepository + ?Sized> CanonicalRunTerminalRepository for T {
    type Error = <T as RemoteSessionRepository>::Error;
    fn read_canonical_run_terminal(&self, conversation_id: &str, run_id: &str)
        -> Result<Option<CanonicalRunTerminal>, Self::Error> {
        self.canonical_run_terminal(conversation_id, run_id)
    }
}
