// SPDX-License-Identifier: Apache-2.0
//! Real source admission for integration fixtures; no optimistic replica Run.
use nola_domain::remote_submission::*;
use nola_store::{AdmittedReplicaTurnInput, BeginTurnInput, NolaStore, StoreResult};
use nola_store_port::run_message_admission::RunMessageAdmissionRepository;

#[path = "remote_submission_from.rs"]
mod remote_submission_from;
use remote_submission_from::RemoteSubmissionFixture as _;

pub trait RemoteSubmissionFixture {
    fn admit_remote_fixture(&mut self, input: &AdmittedReplicaTurnInput) -> StoreResult<()>;
}
impl RemoteSubmissionFixture for NolaStore {
    fn admit_remote_fixture(&mut self, input: &AdmittedReplicaTurnInput) -> StoreResult<()> {
        let directory = tempfile::tempdir().unwrap();
        let mut source = NolaStore::open(directory.path().join("actual-source.sqlite"))?;
        let registered = source.register_source_submission(
            &RemoteSubmissionIdentity {
                source_host_id: input.source_host_id.clone(),
                client_id: "fixture-client".into(),
                conversation_id: input.conversation_id.clone(),
                submission_id: input.run_id.clone(),
                user_message_id: input.user_message_id.clone(),
                assistant_message_id: input.assistant_message_id.clone(),
            },
            &serde_json::json!({"fixture_submission":input.run_id,"user_text":input.user_text}),
        )?;
        source.begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            conversation_id: input.conversation_id.clone(),
            conversation_title: input.conversation_title.clone(),
            user_message_id: input.user_message_id.clone(),
            user_part_id: input.user_part_id.clone(),
            user_text: input.user_text.clone(),
            user_content_json: input.user_content_json.clone(),
            additional_user_parts: vec![],
            assistant_message_id: input.assistant_message_id.clone(),
            assistant_part_id: format!("{}:text", input.assistant_message_id),
            run_id: input.run_id.clone(),
            context_checkpoint_id: None,
            created_at_ms: input.created_at_ms,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![],
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
        })?;
        let through =
            source.local_run_message_admission_head(&input.conversation_id, &input.run_id)?;
        let receipt = source
            .source_submission_receipt(
                &registered.receipt.identity,
                &registered.receipt.request_digest,
            )?
            .unwrap();
        self.admit_remote_fixture_from(input, receipt)?;
        let mut after =
            self.run_message_admission_reception_state(&input.source_host_id, &through)?;
        while after.sequence < through.sequence {
            let page = source.read_run_message_admissions(&after, &through, 64, 262144)?;
            self.begin_write_unit_of_work()?;
            after = self.receive_run_message_admissions(&input.source_host_id, &page)?;
            self.commit_write_unit_of_work()?;
        }
        Ok(())
    }
}
