// SPDX-License-Identifier: Apache-2.0
//! Same-connection remote session adapter.
use crate::{NolaStore, StoreError, StoreResult};
use nola_domain::remote_submission::*;
use nola_store_port::remote_session::*;
impl RemoteSessionRepository for NolaStore {
    type Error = StoreError;
    fn admitted_remote_run_identity(&self, run: &str) -> StoreResult<Option<RemoteSubmissionIdentity>> {
        NolaStore::admitted_remote_run_identity(self, run)
    }
    fn observed_source_submission_receipt(
        &self,
        host: &str,
        client: &str,
        conversation: &str,
        run: &str,
        assistant: &str,
    ) -> StoreResult<Option<SourceSubmissionReceipt>> {
        NolaStore::observed_source_submission_receipt(
            self,
            host,
            client,
            conversation,
            run,
            assistant,
        )
    }
    fn adopt_observed_remote_run(&mut self, receipt: &SourceSubmissionReceipt) -> StoreResult<()> {
        NolaStore::adopt_observed_remote_run(self, receipt)
    }
    fn abandon_prepared_remote_submission(
        &mut self,
        identity: &RemoteSubmissionIdentity,
    ) -> StoreResult<RemoteSubmissionIntentView> {
        NolaStore::abandon_prepared_remote_submission(self, identity)
    }
    fn canonical_run_terminal(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<CanonicalRunTerminal>, Self::Error> {
        NolaStore::canonical_run_terminal(self, conversation_id, run_id)
    }
    fn prepare_remote_submission(
        &mut self,
        input: &PrepareRemoteSubmissionInput,
    ) -> Result<RemoteSubmissionIntentView, Self::Error> {
        NolaStore::prepare_remote_submission(self, input)
    }
    fn bind_remote_submission_request(
        &mut self,
        id: &RemoteSubmissionIdentity,
        request: &serde_json::Value,
    ) -> Result<RemoteSubmissionIntentView, Self::Error> {
        NolaStore::bind_remote_submission_request(self, id, request)
    }
    fn remote_submission_intent(
        &self,
        conversation: &str,
    ) -> Result<Option<RemoteSubmissionIntentView>, Self::Error> {
        NolaStore::remote_submission_intent(self, conversation)
    }
    fn remote_submission_request(
        &self,
        id: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> Result<serde_json::Value, Self::Error> {
        NolaStore::remote_submission_request(self, id, digest)
    }
    fn register_source_submission(
        &mut self,
        id: &RemoteSubmissionIdentity,
        request: &serde_json::Value,
    ) -> Result<RegisteredSourceSubmission, Self::Error> {
        NolaStore::register_source_submission(self, id, request)
    }
    fn exact_bound_remote_submission(
        &self,
        id: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> Result<RemoteSubmissionIntentView, Self::Error> {
        NolaStore::exact_bound_remote_submission(self, id, digest)
    }
    fn source_submission_receipt(
        &self,
        id: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> Result<Option<SourceSubmissionReceipt>, Self::Error> {
        NolaStore::source_submission_receipt(self, id, digest)
    }
    fn settle_source_submission(
        &mut self,
        id: &RemoteSubmissionIdentity,
        digest: &str,
        rejection: Option<&str>,
    ) -> Result<SourceSubmissionReceipt, Self::Error> {
        NolaStore::settle_source_submission(self, id, digest, rejection)
    }
    fn accept_remote_submission_receipt(
        &mut self,
        receipt: &SourceSubmissionReceipt,
    ) -> Result<RemoteSubmissionIntentView, Self::Error> {
        NolaStore::accept_remote_submission_receipt(self, receipt)
    }
    fn assistant_message_id_for_run(
        &self,
        run_id: &str,
        conversation_id: &str,
    ) -> Result<Option<String>, Self::Error> {
        NolaStore::assistant_message_id_for_run(self, run_id, conversation_id)
    }
}
impl SessionProjectionRepository for NolaStore {
    type Error = StoreError;
    fn load_authoritative_session(
        &self,
        id: &str,
    ) -> Result<Option<AuthoritativeSessionRecord>, Self::Error> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        if self.has_remote_presentation_owner(id)? {
            return Ok(None);
        }
        Ok(self
            .load_local_session_recovery(id)?
            .map(AuthoritativeSessionRecord::Local))
    }
}

impl NolaStore {
    pub(crate) fn load_local_session_recovery(
        &self,
        id: &str,
    ) -> Result<Option<StoredLocalSessionRecovery>, StoreError> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        use nola_store_port::run_effect::RunEffectRepository;
        let Some(recovery) = self.load_recovery_checkpoint(id)? else {
            return Ok(None);
        };
        let mut invocation = None;
        let mut delegated_invocations = Vec::new();
        if !recovery.replay_inputs.is_empty() {
            let base = recovery.replay_base.as_ref().ok_or_else(|| {
                StoreError::Invariant("session replay tail has no Run owner".into())
            })?;
            invocation = self.load_run_invocation_config(&base.run_id)?;
            let mut query = self.connection.prepare_cached(
                "SELECT DISTINCT d.record_json FROM core_checkpoint_bodies c,
                     json_each(c.checkpoint_json,'$.state.host.pending_host_effects.effects') e
                     JOIN delegated_invocations d ON d.effect_id=json_extract(e.value,'$.effect_id')
                         OR d.effect_id=json_extract(e.value,'$.target_effect_id')
                     WHERE c.conversation_id=?1 AND d.run_id=?2",
            )?;
            let rows = query.query_map(rusqlite::params![id, base.run_id], |row| {
                row.get::<_, String>(0)
            })?;
            for row in rows {
                delegated_invocations.push(serde_json::from_str(&row?).map_err(|error| {
                    StoreError::Invariant(format!("invalid session restore evidence: {error}"))
                })?);
            }
        }
        Ok(Some(StoredLocalSessionRecovery {
            recovery,
            invocation,
            delegated_invocations,
        }))
    }
}
