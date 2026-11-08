// SPDX-License-Identifier: Apache-2.0
//! One room coordinator request, using the canonical auxiliary attempt ledger.
use crate::{application_port::CliApplicationPort, llm::{LlmConfig, AuxiliaryPhysicalOutcome}};
use nola_application::{ApplicationCommand, agent_room::{AgentRoomCommand, RoomSnapshot, RoomSubmission, RoomRoutingDecision, routing_messages}};
use nola_domain::*;
use serde_json::json;
use sha2::{Digest, Sha256};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64).unwrap_or(0)
}

pub(crate) fn route(
    app: &mut CliApplicationPort, room: &RoomSnapshot, submission: &RoomSubmission,
    config: &LlmConfig, profile_id: &str, profile_revision: u64,
) -> Result<RoomSubmission, String> {
    let messages = routing_messages(room, submission)?;
    let operation_id = format!("room-route:{}:{}", room.room.id(), submission.input.submission_id);
    let attempt_id = format!("{operation_id}:physical");
    let configuration = AuxiliaryInvocationConfiguration {
        provider: format!("configured:{profile_id}"), model: Some(config.model.clone()),
        configuration_revision: profile_revision,
        configuration_digest: crate::llm::request_configuration_identity(config),
        input_digest: format!("{:x}", Sha256::digest(serde_json::to_vec(&messages).map_err(|e| e.to_string())?)),
        max_attempts: 1, maximum_duration_ms: 60_000,
        max_output_tokens_per_attempt: Some(config.max_output_tokens),
        total_output_token_budget: Some(config.max_output_tokens),
    };
    let admitted: AuxiliaryInvocationReceipt = app.execute(ApplicationCommand::AuxiliaryInvocation {
        command: AuxiliaryInvocationCommand::Admit {
            operation_id: operation_id.clone(),
            owner: AuxiliaryInvocationOwner::Room { room_id: room.room.id().into() },
            scenario: AuxiliaryInvocationScenario::RoomRouting, configuration, at_ms: now(),
        },
    })?;
    // A completed physical request survives a lost response or process restart.
    // Consume its recorded decision rather than issuing another model request.
    if let Some(attempt) = admitted.attempt.as_ref().filter(|a| a.state == ModelAttemptState::Succeeded) {
        let decision: RoomRoutingDecision = serde_json::from_value(attempt.output.clone().ok_or("room_routing_result_missing")?)
            .map_err(|_| "room_routing_invalid_result")?;
        return app.execute(ApplicationCommand::AgentRoom { command: AgentRoomCommand::SelectRecipient {
            room_id: room.room.id().into(), submission_id: submission.input.submission_id.clone(),
            expected_revision: submission.revision, participant_id: decision.participant_id, at_ms: now(),
        }});
    }
    let started: AuxiliaryInvocationReceipt = app.execute(ApplicationCommand::AuxiliaryInvocation {
        command: AuxiliaryInvocationCommand::StartAttempt {
            operation_id: operation_id.clone(), attempt_id: attempt_id.clone(),
            expected_revision: admitted.operation.revision, at_ms: now(),
        },
    })?;
    if !started.dispatch_permitted { return Err("room_routing_already_dispatched_inspect_before_retry".into()); }
    let attempt = started.attempt.as_ref().ok_or("room_routing_attempt_missing")?;
    let (content, usage) = match crate::llm::invoke_auxiliary_once(config, &messages, 60_000) {
        Ok(AuxiliaryPhysicalOutcome::Complete { content, usage, .. }) => (content, usage),
        Ok(AuxiliaryPhysicalOutcome::Unknown) => return Err("room_routing_result_unknown".into()),
        Err(_) => (Err("room_routing_transport_rejected".into()), json!({"coverage":"unknown","provider_raw":null,"normalized":null})),
    };
    let decision = content.ok().and_then(|text| serde_json::from_str::<RoomRoutingDecision>(&text).ok())
        .filter(|decision| room.room.recipient(&decision.participant_id).is_ok());
    let output = decision.as_ref().map(|decision| json!({"participant_id":decision.participant_id}));
    let _: AuxiliaryInvocationReceipt = app.execute(ApplicationCommand::AuxiliaryInvocation {
        command: AuxiliaryInvocationCommand::RecordReceipt {
            operation_id, attempt_id, expected_attempt_revision: attempt.revision,
            state: if decision.is_some() { ModelAttemptState::Succeeded } else if config.cancellation.is_requested() { ModelAttemptState::Cancelled } else { ModelAttemptState::Failed },
            usage, output,
            failure: decision.is_none().then(|| ModelAttemptFailure { code:"room_routing_invalid_result".into(), message:"Coordinator did not return an active participant".into(),retryable:false,http_status:None }),
            at_ms:now(),
        },
    })?;
    let decision = decision.ok_or("room_routing_invalid_result")?;
    app.execute(ApplicationCommand::AgentRoom { command: AgentRoomCommand::SelectRecipient {
        room_id: room.room.id().into(), submission_id: submission.input.submission_id.clone(),
        expected_revision: submission.revision, participant_id: decision.participant_id, at_ms: now(),
    }})
}
