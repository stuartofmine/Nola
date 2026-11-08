// SPDX-License-Identifier: Apache-2.0
//! Bind an integration fixture to an already committed source receipt.
use nola_domain::remote_submission::{PrepareRemoteSubmissionInput, SourceSubmissionReceipt};
use nola_store::{AdmittedReplicaTurnInput, NolaStore, StoreResult};

pub trait RemoteSubmissionFixture {
    fn admit_remote_fixture_from(
        &mut self,
        input: &AdmittedReplicaTurnInput,
        receipt: SourceSubmissionReceipt,
    ) -> StoreResult<()>;
}
impl RemoteSubmissionFixture for NolaStore {
    fn admit_remote_fixture_from(
        &mut self,
        input: &AdmittedReplicaTurnInput,
        receipt: SourceSubmissionReceipt,
    ) -> StoreResult<()> {
        let prepared = self.prepare_remote_submission(&PrepareRemoteSubmissionInput {
            source_host_id: input.source_host_id.clone(),
            client_id: "fixture-client".into(),
            conversation_id: input.conversation_id.clone(),
            conversation_title: input.conversation_title.clone(),
            submission_id: input.run_id.clone(),
            user_message_id: input.user_message_id.clone(),
            user_part_id: input.user_part_id.clone(),
            user_text: input.user_text.clone(),
            user_content_json: input.user_content_json.clone(),
            assistant_message_id: input.assistant_message_id.clone(),
            assistant_part_id: format!("{}:text", input.assistant_message_id),
            context_checkpoint_id: input.context_checkpoint_id.clone(),
            created_at_ms: input.created_at_ms,
        })?;
        let bound = self.bind_remote_submission_request(
            &prepared.identity,
            &serde_json::json!({"fixture_submission":input.run_id,"user_text":input.user_text}),
        )?;
        self.begin_write_unit_of_work()?;
        assert_eq!(bound.identity, receipt.identity);
        assert_eq!(
            bound.request_digest.as_deref(),
            Some(receipt.request_digest.as_str())
        );
        self.accept_remote_submission_receipt(&receipt)?;
        self.commit_write_unit_of_work()?;
        Ok(())
    }
}
