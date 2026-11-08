// SPDX-License-Identifier: Apache-2.0
use nola_domain::run_message_admission::*;
pub trait RunMessageAdmissionRepository {
    type Error: std::fmt::Display;
    fn local_run_context_usage(
        &self,
        identity: &nola_domain::assistant_journal::AssistantJournalIdentity,
    ) -> Result<
        Option<nola_domain::committed_run_context_usage::CommittedRunContextUsage>,
        Self::Error,
    >;
    fn accept_remote_run_context_usage(
        &mut self,
        host: &str,
        record: &nola_domain::committed_run_context_usage::CommittedRunContextUsage,
    ) -> Result<nola_domain::committed_run_context_usage::CommittedRunContextUsage, Self::Error>;
    fn local_run_message_admission_head(
        &self,
        conversation: &str,
        run: &str,
    ) -> Result<RunMessageAdmissionCursor, Self::Error>;
    fn read_run_message_admissions(
        &self,
        after: &RunMessageAdmissionCursor,
        through: &RunMessageAdmissionCursor,
        max_records: usize,
        max_encoded_bytes: usize,
    ) -> Result<RunMessageAdmissionPage, Self::Error>;
    fn run_message_admission_reception_state(
        &self,
        host: &str,
        through: &RunMessageAdmissionCursor,
    ) -> Result<RunMessageAdmissionCursor, Self::Error>;
    fn receive_run_message_admissions(
        &mut self,
        host: &str,
        page: &RunMessageAdmissionPage,
    ) -> Result<RunMessageAdmissionCursor, Self::Error>;
}
