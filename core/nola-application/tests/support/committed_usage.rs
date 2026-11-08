// SPDX-License-Identifier: Apache-2.0
//! Convenience admission for usage fixtures without streaming observations.
#[path = "committed_usage_stream.rs"]
mod stream;
use nola_domain::committed_run_context_usage::CommittedRunContextUsage;
use nola_domain::remote_submission::SourceSubmissionReceipt;
use nola_store::NolaStore;
use stream::seed_stream_with_observer;

#[allow(dead_code)] // Only the multi-Run receiver regression needs a second admission.
pub fn seed_another_run(store: &mut NolaStore, tokens: usize) -> (CommittedRunContextUsage, SourceSubmissionReceipt) {
    stream::seed_stream_with_identity(store, Some(tokens), 0, 0, |_| {},
        nola_domain::remote_submission::RemoteSubmissionIdentity {
            source_host_id: "usage-host".into(), client_id: "usage-client".into(), conversation_id: "usage".into(),
            submission_id: "usage-run-two".into(), user_message_id: "usage-user-two".into(), assistant_message_id: "usage-assistant-two".into(),
        })
}

pub fn seed(
    store: &mut NolaStore,
    tokens: usize,
) -> (CommittedRunContextUsage, SourceSubmissionReceipt) {
    seed_with_usage(store, Some(tokens))
}
pub fn seed_with_usage(
    store: &mut NolaStore,
    tokens: Option<usize>,
) -> (CommittedRunContextUsage, SourceSubmissionReceipt) {
    seed_stream_with_observer(store, tokens, 0, 0, |_| {})
}
