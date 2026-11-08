// SPDX-License-Identifier: Apache-2.0
//! Initial bounded control facts, not a second copy of input, Parts or config.
use nola_agent_engine::agent::protocol::AgentEvent;
use nola_domain::core_event::CoreEventEnvelope;
use nola_store_port::run_creation::InitialCoreRunEvent;

pub(super) fn plan(
    events: &[CoreEventEnvelope<AgentEvent>],
) -> Result<Vec<InitialCoreRunEvent>, String> {
    events
        .iter()
        .filter_map(|envelope| {
            // Only authoritative bounded lifecycle facts belong in this ledger.
            // Input/Part bodies already have canonical message/journal ownership;
            // catalog, schemas, captured context and cumulative control projections
            // remain in their existing transactional repositories/checkpoint.
            let payload = match &envelope.event {
                AgentEvent::AssistantActivityBound { .. } | AgentEvent::AssistantEffectBound { .. } | AgentEvent::AssistantSegmentStarted { .. }
                | AgentEvent::InputAccepted { .. } | AgentEvent::InputWithdrawn { .. }
                | AgentEvent::RequirementScopeChanged { .. }
                | AgentEvent::RequirementAssessed { .. }
                | AgentEvent::RunResultDecided { .. }
                | AgentEvent::RuntimeDecisionFactObserved { .. }
                | AgentEvent::UserInputWaitChanged { .. } => {
                    match serde_json::to_value(&envelope.event) {
                        Ok(payload) => payload,
                        Err(error) => return Some(Err(error.to_string())),
                    }
                }
                AgentEvent::InputApplied { turn_id, applications, .. } => {
                    // Bodies already have canonical message ownership; retain
                    // exact application identities, not another input copy.
                    serde_json::json!({"type":"input_applied","turn_id":turn_id,"applications":applications})
                }
                AgentEvent::ContextUpdated {
                    measurement,
                    prompt_cache_measurement,
                    prompt_cache_diagnostic,
                    used_tokens,
                    context_window_tokens,
                    usage_percent,
                    usage_is_estimated,
                    context_usage,
                    ..
                } => serde_json::json!({
                    "type":"context_updated", "measurement":measurement,
                    "prompt_cache_measurement":prompt_cache_measurement,
                    "prompt_cache_diagnostic":prompt_cache_diagnostic, "used_tokens":used_tokens,
                    "context_window_tokens":context_window_tokens,"usage_percent":usage_percent,
                    "usage_is_estimated":usage_is_estimated,"context_usage":context_usage
                }),
                AgentEvent::SessionStarted { .. }
                | AgentEvent::TurnStarted { .. }
                | AgentEvent::HostCatalogUpdated { .. }
                | AgentEvent::SessionConfigurationUpdated { .. }
                | AgentEvent::CompactionRequested { .. } => {
                    match serde_json::to_value(&envelope.event) {
                        Ok(payload) => payload,
                        Err(error) => return Some(Err(error.to_string())),
                    }
                }
                AgentEvent::CompactionCompleted { replaced_item_count: 0, compaction_item_id: None, .. }
                | AgentEvent::TurnCompleted { .. } => {
                    match serde_json::to_value(&envelope.event) {
                        Ok(payload) => payload,
                        Err(error) => return Some(Err(error.to_string())),
                    }
                }
                _ => return None,
            };
            Some((|| {
                let event_kind = payload
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("initial Core event has no type")?
                    .to_owned();
                let payload_json =
                    serde_json::to_string(&payload).map_err(|error| error.to_string())?;
                if payload_json.len() > 16 * 1024 {
                    return Err("initial bounded Core control fact exceeds its byte budget".into());
                }
                Ok(InitialCoreRunEvent {
                    core_session_id: envelope.session_id.clone(),
                    core_turn_id: envelope.turn_id.clone(),
                    core_cursor: envelope.cursor.sequence,
                    core_revision: envelope.revision,
                    event_kind,
                    payload_json,
                })
            })())
        })
        .collect()
}
