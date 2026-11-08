// SPDX-License-Identifier: Apache-2.0
//! Exact read-only evidence. Missing/transport loss never means not submitted.
use nola_domain::{
    RunRecoveryInconsistency as Inconsistency, RunRecoveryInspection,
    RunRecoveryObservation as Observation,
};
use nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository;

pub fn inspect_run_recovery<R: RunRecoveryInspectionRepository + ?Sized>(
    store: &R,
    conversation_id: String,
    expected_run_id: String,
    assistant_message_id: String,
) -> Result<RunRecoveryInspection, String> {
    if [&conversation_id, &expected_run_id, &assistant_message_id]
        .iter()
        .any(|id| id.trim().is_empty())
    {
        return Err("exact Run recovery inspection requires all three identities".into());
    }
    let observation = match store
        .inspect_exact_run(&expected_run_id)
        .map_err(|e| e.to_string())?
    {
        None => Observation::Missing,
        Some(record)
            if record.conversation_id != conversation_id
                || record.assistant_message_id.as_deref()
                    != Some(assistant_message_id.as_str())
                || record.kind != "conversation" =>
        {
            Observation::IdentityMismatch
        }
        Some(record) if !record.assistant_owner_valid => Observation::Inconsistent {
            reason: Inconsistency::AssistantOwnerInvalid,
        },
        Some(record) => {
            match (record.terminal, record.completed_at_ms) {
                (Some(terminal), Some(completed_at_ms)) => Observation::Terminal {
                    result: terminal.result,
                    result_digest: terminal.result_digest,
                    completed_at_ms,
                },
                (Some(_), None) => Observation::Inconsistent {
                    reason: Inconsistency::TerminalTimestampMissing,
                },
                (None, Some(_)) => Observation::Inconsistent {
                    reason: Inconsistency::NonterminalHasCompletionTimestamp,
                },
                (None, None) => Observation::Active {
                    run_status: record.status,
                    checkpoint_present: record.checkpoint_present,
                    pending_effect_count: record.pending_effect_count,
                    source_host_id: record.source_host_id,
                    has_session_lease: record.has_session_lease,
                    deliveries: record.deliveries,
                },
            }
        }
    };
    Ok(RunRecoveryInspection {
        conversation_id,
        run_id: expected_run_id,
        assistant_message_id,
        observation,
    })
}
