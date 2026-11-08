// SPDX-License-Identifier: Apache-2.0
//! Isolated integration-test composition. Production adapters live in bootstrap.
/// SQLite is chosen only by integration-test composition, never by the production Runtime type.
pub type ApplicationRuntime = nola_application::ApplicationRuntime<nola_store::NolaStore>;
#[allow(dead_code)]
pub mod admission_frontier;
#[allow(dead_code)]
pub mod agent_task_admission;
#[allow(dead_code)]
pub mod model_identity;

pub fn open_runtime(path: impl AsRef<std::path::Path>) -> Result<ApplicationRuntime, String> {
    let path = path.as_ref();
    let store = nola_store::NolaStore::open(path).map_err(|error| error.to_string())?;
    let uploads = Box::new(
        nola_store::content_upload_port::LazyContentUploadStore::for_application(
            path,
            nola_application::MAXIMUM_CONTENT_UPLOAD_BYTES,
        ),
    );
    Ok(nola_application::ApplicationRuntime::from_store(
        store, uploads,
    ))
}

/// Current state is the production cold materialization of baseline + typed
/// replay facts, never the older bytes kept in core_checkpoint_bodies.
#[allow(dead_code)]
pub fn current_checkpoint(
    path: impl AsRef<std::path::Path>,
    conversation: &str,
) -> nola_application::ConversationCheckpoint {
    let mut runtime = open_runtime(path).unwrap();
    let current: Option<nola_application::ConversationCheckpoint> = serde_json::from_value(
        runtime
            .execute(nola_application::ApplicationCommand::LoadCheckpoint {
                conversation_id: conversation.into(),
            })
            .unwrap(),
    )
    .unwrap();
    current.expect("exact conversation has a current materialized checkpoint")
}

/// Fixture provider admission uses the same transaction as production before
/// any test emits model content. Returns the newly committed delivery cursor.
#[allow(dead_code)]
pub fn admit_model_sampling(
    store: &mut nola_store::NolaStore,
    run_id: &str,
    digest: &str,
    request: &mut nola_agent_engine::agent::host::HostRequestEnvelope,
    now: i64,
) {
    let conversation_id = request.session_id.clone();
    admit_model_sampling_for_conversation(store, &conversation_id, run_id, digest, request, now)
}

#[allow(dead_code)]
pub fn admit_model_sampling_for_conversation(
    store: &mut nola_store::NolaStore,
    conversation_id: &str,
    run_id: &str,
    digest: &str,
    request: &mut nola_agent_engine::agent::host::HostRequestEnvelope,
    now: i64,
) {
    use nola_agent_engine::agent::host::*;
    let claimed = nola_application::claim_committed_host_requests(
        store,
        nola_application::ClaimCommittedHostRequests {
            run_id: run_id.into(),
            worker_generation: format!("fixture-generation-{}", uuid::Uuid::new_v4()),
            lease_owner: format!("fixture-owner-{}", uuid::Uuid::new_v4()),
            claimed_at_ms: now.saturating_sub(2),
            lease_expires_at_ms: now.saturating_add(60_000),
            limit: 64,
        },
    )
    .unwrap();
    let mut claimed_request = claimed
        .requests
        .into_iter()
        .find(|candidate| candidate.effect_id == request.effect_id)
        .expect("fixture must claim its pending model effect");
    let start_sequence = claimed_request.resume_after_sequence.unwrap_or(0) + 1;
    nola_application::reduce_and_accept_host_response(
        store,
        nola_application::ReduceAndAcceptHostResponse {
            conversation_id: conversation_id.into(),
            run_id: run_id.into(),
            invocation_config_digest: digest.into(),
            received_at_ms: now.saturating_sub(1),
            response: HostResponseEnvelope {
                request_id: claimed_request.request_id.clone(),
                attempt_id: claimed_request.attempt_id.clone(),
                delegated_invocation_id: claimed_request.delegated_invocation_id.clone(),
                effect_id: claimed_request.effect_id.clone(),
                host_id: claimed_request.host_id.clone(),
                lease_id: claimed_request.lease_id.clone(),
                session_id: claimed_request.session_id.clone(),
                turn_id: claimed_request.turn_id.clone(),
                catalog_revision: claimed_request.catalog_revision,
                availability_epoch: claimed_request.availability_epoch,
                delivery: claimed_request.delivery.clone(),
                sequence: start_sequence,
                payload: HostResponse::ExecutionStarted,
            },
        },
    )
    .unwrap();
    claimed_request.resume_after_sequence = Some(start_sequence);
    let sampling_sequence = start_sequence + 1;
    nola_application::reduce_and_accept_host_response(
        store,
        nola_application::ReduceAndAcceptHostResponse {
            conversation_id: conversation_id.into(),
            run_id: run_id.into(),
            invocation_config_digest: digest.into(),
            received_at_ms: now,
            response: HostResponseEnvelope {
                request_id: claimed_request.request_id.clone(),
                attempt_id: claimed_request.attempt_id.clone(),
                delegated_invocation_id: claimed_request.delegated_invocation_id.clone(),
                effect_id: claimed_request.effect_id.clone(),
                host_id: claimed_request.host_id.clone(),
                lease_id: claimed_request.lease_id.clone(),
                session_id: claimed_request.session_id.clone(),
                turn_id: claimed_request.turn_id.clone(),
                catalog_revision: claimed_request.catalog_revision,
                availability_epoch: claimed_request.availability_epoch,
                delivery: claimed_request.delivery.clone(),
                sequence: sampling_sequence,
                payload: HostResponse::ModelAttemptStarted {
                    provider: "fixture-provider".into(),
                    model: "test-model".into(),
                    reason: "initial".into(),
                },
            },
        },
    )
    .unwrap();
    let delivery = claimed_request.delivery;
    *request = nola_application::load_committed_host_requests(store, run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|r| r.effect_id == request.effect_id)
        .unwrap();
    request.delivery = delivery;
}

#[allow(dead_code)]
pub mod goal_execution;
