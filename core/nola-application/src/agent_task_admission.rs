// SPDX-License-Identifier: Apache-2.0
//! Export committed Core admission from its source Store; import only over trusted Host IPC.
use nola_domain::agent_task_admission::{
    AgentTaskAdmission, RuntimeAgentTaskAuthority, AGENT_TASK_ADMISSION_VERSION,
};
use nola_agent_engine::agent::{
    host::{HostResponse, HostResponseEnvelope},
    task::{TaskKind, TaskState},
    AgentEvent,
};
use nola_store_port::runtime_task::RuntimeTaskRepository;
use nola_domain::core_event::CoreEventSource;

pub(crate) fn export<R: RuntimeTaskRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
    run_id: &str,
    task_id: &str,
    event_cursor: u64,
) -> Result<AgentTaskAdmission, String> {
    let evidence = store
        .source_agent_task_admission_evidence(conversation_id, run_id, task_id, event_cursor)
        .map_err(|error| format!("read committed Task admission failed: {error}"))?
        .ok_or("committed Task admission source is missing")?;
    if evidence.core_event_json.len() > 65_536 || evidence.host_response_json.len() > 65_536 {
        return Err("Task admission source exceeds bounded evidence contract".into());
    }
    let mut event: serde_json::Value = serde_json::from_str(&evidence.core_event_json)
        .map_err(|error| format!("decode Task admission event failed: {error}"))?;
    let identity: CoreEventSource = serde_json::from_value(
        event
            .as_object_mut()
            .and_then(|object| object.remove("core_envelope"))
            .ok_or("Task admission has no Core envelope")?,
    )
    .map_err(|error| format!("decode Task admission Core envelope failed: {error}"))?;
    let event: AgentEvent = serde_json::from_value(event)
        .map_err(|error| format!("decode Task admission Core event failed: {error}"))?;
    let AgentEvent::TaskUpdated { task, .. } = event else {
        return Err("Task admission source is not TaskUpdated".into());
    };
    let response: HostResponseEnvelope = serde_json::from_str(&evidence.host_response_json)
        .map_err(|error| format!("decode Task admission Host response failed: {error}"))?;
    let HostResponse::TaskPromoted {
        task_id: promoted_id,
        kind,
        state,
        lineage,
        input_watermark,
        ..
    } = &response.payload
    else {
        return Err("Task admission source is not TaskPromoted".into());
    };
    let turn_id = identity
        .turn_id
        .as_deref()
        .ok_or("Task admission has no owning Turn")?;
    if identity.cursor.sequence != event_cursor
        || identity.revision == 0
        || task.id != task_id
        || promoted_id != task_id
        || task.kind != TaskKind::Agent
        || *kind != TaskKind::Agent
        || task.state != TaskState::Queued
        || *state != TaskState::Queued
        || task.owner_session_id.0 != identity.session_id
        || task.owner_turn_id.0 != turn_id
        || response.session_id != identity.session_id
        || response.turn_id.as_deref() != Some(turn_id)
        || task.lineage != *lineage
        || input_watermark.is_none()
        || task.durability.input_watermark != *input_watermark
        || evidence.source_transition_id != format!("transition:{}", evidence.source_host_event_id)
    {
        return Err("committed Task admission identity mismatch".into());
    }
    AgentTaskAdmission {
        protocol_version: AGENT_TASK_ADMISSION_VERSION,
        task_id: task_id.into(),
        parent_conversation_id: conversation_id.into(),
        parent_core_session_id: identity.session_id,
        predecessor_task_id: task.lineage.resumed_from_task_id.clone(),
        parent_run_id: run_id.into(),
        parent_turn_id: turn_id.into(),
        child_conversation_id: task
            .lineage
            .child_session_id
            .ok_or("Task admission has no child conversation")?
            .0,
        parent_tool_call_id: task
            .lineage
            .parent_call_id
            .ok_or("Task admission has no parent call")?
            .0,
        parent_effect_id: response.effect_id.0,
        input_watermark: task.durability.input_watermark,
        source_store_epoch: evidence.source_store_epoch,
        source_transition_id: evidence.source_transition_id,
        source_host_event_id: evidence.source_host_event_id,
        source_checkpoint_revision: evidence.source_checkpoint_revision,
        core_revision: identity.revision,
        core_event_cursor: identity.cursor.sequence,
        admission_digest: String::new(),
    }
    .seal()
}

pub(crate) fn import<R: RuntimeTaskRepository + ?Sized>(
    store: &mut R,
    admission: AgentTaskAdmission,
) -> Result<RuntimeAgentTaskAuthority, String> {
    // ApplicationRuntime supplies the outer publication UoW. This entry point is
    // reserved for authenticated Host transport, never ordinary model commands.
    store
        .admit_runtime_agent_task(&admission)
        .map_err(|error| format!("import committed Task admission failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::core_event::{CoreEventEnvelope, EventCursor};

    #[test]
    fn persisted_queued_task_retains_actual_core_identity_without_changing_event() {
        let event: AgentEvent = serde_json::from_value(serde_json::json!({
            "type": "task_updated", "task_projection_revision": 13,
            "task": {"id": "task", "owner_session_id": "actual-core-session",
                "owner_turn_id": "actual-turn", "kind": "agent", "state": "queued"}
        }))
        .unwrap();
        let envelope = CoreEventEnvelope {
            session_id: "actual-core-session".into(),
            turn_id: Some("actual-turn".into()),
            revision: 47,
            cursor: EventCursor { sequence: 103 },
            event,
        };
        let mut payload = crate::committed_event_payload_for_persistence(&envelope).unwrap();
        let identity: CoreEventSource = serde_json::from_value(
            payload
                .as_object_mut()
                .unwrap()
                .remove("core_envelope")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(identity.session_id, envelope.session_id);
        assert_eq!(identity.turn_id, envelope.turn_id);
        assert_eq!(identity.revision, 47);
        assert_eq!(identity.cursor.sequence, 103);
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<AgentEvent>(payload).unwrap()).unwrap(),
            serde_json::to_value(&envelope.event).unwrap()
        );
    }
}
