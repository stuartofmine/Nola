// SPDX-License-Identifier: Apache-2.0
//! Exact pre-reduction binding and clock facts, with no copied request payload.
use crate::*;
use nola_store_port::host_replay::{
    CoreHostBindingOverlay, HostReplayReduction, HOST_REPLAY_FACTS_VERSION,
};

pub(crate) fn encode_reduction(
    response: &HostResponseEnvelope,
    binding: Option<CoreHostBindingOverlay>,
    decision_time_ms: i64,
) -> Result<String, String> {
    let normalized_response = serde_json::value::RawValue::from_string(
        serde_json::to_string(response).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    serde_json::to_string(&HostReplayReduction::Replayable {
        protocol_version: HOST_REPLAY_FACTS_VERSION,
        decision_time_ms,
        normalized_response,
        binding,
    })
    .map_err(|error| error.to_string())
}

pub(crate) fn decode_reduction(
    input: &nola_store_port::run_effect::StoredCoreReplayInput,
) -> Result<(HostResponseEnvelope, Option<CoreHostBindingOverlay>, u64), String> {
    let facts: HostReplayReduction = serde_json::from_str(&input.reduction_json)
        .map_err(|error| format!("decode typed Core replay facts failed: {error}"))?;
    facts
        .validate_clock(input.received_at_ms)
        .map_err(str::to_owned)?;
    match facts {
        HostReplayReduction::Replayable {
            decision_time_ms, normalized_response, binding, ..
        } => Ok((
            serde_json::from_str(normalized_response.get())
                .map_err(|error| format!("decode normalized Core replay response failed: {error}"))?,
            binding,
            crate::decision_time_ms(decision_time_ms)?,
        )),
        HostReplayReduction::BaselineCovered { .. } =>
            Err("Core replay tail unexpectedly contains baseline-covered history".into()),
        HostReplayReduction::RecoveryRequired { reason, .. } =>
            Err(format!("Core replay recovery required: {reason}; historical binding evidence is unavailable; automatic execution is refused")),
    }
}
