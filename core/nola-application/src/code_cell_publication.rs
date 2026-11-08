// SPDX-License-Identifier: Apache-2.0
//! Publish cell observation facts in the owning assistant transaction.
use nola_agent_engine::agent::{parts::AgentPart, AgentThreadState};
use nola_store_port::code_cell::CodeCellRecord;

pub(crate) fn read_for_effect<S: crate::repositories::ConversationCommandRepositories + ?Sized>(
    store: &S, request: &nola_agent_engine::agent::host::HostRequestEnvelope,
) -> Result<Option<nola_domain::code_mode::CodeCellView>, String> {
    use nola_agent_engine::agent::host::HostRequest;
    let HostRequest::ReadCodeCellHistory {cell_id, after_output_sequence, ..} = &request.payload
        else { return Err("not a historical code cell observation request".into()); };
    let identity = store.outbox_effect_identity(&request.effect_id.0).map_err(|e| e.to_string())?
        .ok_or("historical observation is not committed")?;
    if identity.status != "pending" || identity.effect_kind != "read_code_cell_history" {
        return Err("historical observation is no longer pending".into());
    }
    let selected = std::collections::HashSet::from([request.effect_id.0.clone()]);
    let snapshot = store.load_selected_host_delivery_witness(&identity.run_id, Some(&selected))
        .map_err(|e| e.to_string())?;
    let conversation = snapshot.conversation_id.ok_or("historical observation has no conversation owner")?;
    let (requests, _) = crate::load_selected_committed_host_requests(store, &identity.run_id, Some(&selected))?;
    let frozen = requests.requests.iter().find(|value| value.effect_id == request.effect_id)
        .ok_or("historical observation is absent from the committed journal")?;
    if frozen.request_id != request.request_id || frozen.session_id != request.session_id
        || frozen.turn_id != request.turn_id || frozen.host_id != request.host_id
        || frozen.attempt_id != request.attempt_id
        || frozen.delegated_invocation_id != request.delegated_invocation_id
        || frozen.lease_id != request.lease_id || frozen.catalog_revision != request.catalog_revision
        || frozen.availability_epoch != request.availability_epoch
        || serde_json::to_value(&frozen.payload).map_err(|e|e.to_string())?
            != serde_json::to_value(&request.payload).map_err(|e|e.to_string())? {
        return Err("historical observation differs from its committed request".into());
    }
    let epoch = store.assistant_source_epoch().map_err(|e|e.to_string())?;
    store.read_code_cell(&nola_store_port::code_cell::CodeCellQuery {
        store_epoch:epoch, conversation_id:conversation, cell_id:cell_id.clone(),
        after_output_sequence:*after_output_sequence,
    }).map_err(|e| e.to_string())
}

pub(crate) fn publish<S: crate::repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut S, conversation: &str, state: &AgentThreadState,
) -> Result<(), String> {
    let Some(turn) = state.execution.active_turn.as_ref() else { return Ok(()); };
    if turn.code_cells.is_empty() { return Ok(()); }
    let epoch = store.assistant_source_epoch().map_err(|error| format!("publish code cell: read store epoch: {error}"))?;
    let owner = store.assistant_owner_for_core_turn(conversation, turn.turn_id.as_str())
        .map_err(|error| format!("publish code cell: resolve Turn owner: {error}"))?.ok_or("code cell lacks accepted Turn owner")?;
    for cell in turn.code_cells.values() {
        if cell.identity().turn_id != turn.turn_id.as_str()
            || cell.identity().conversation_id != state.session_id.as_str() {
            return Err("code cell differs from its Core owner".into());
        }
        let parent = state.conversation.projection.parts.values().find_map(|part| match part {
            AgentPart::Tool(part) if part.call_id == cell.identity().parent_call_id
                && part.ownership.turn_id == turn.turn_id.as_str() => Some(part),
            _ => None,
        }).ok_or("code cell has no canonical parent Tool Part")?;
        let bound = store.assistant_owner_for_part(conversation, &owner.run_id, &parent.id)
            .map_err(|error| format!("publish code cell: resolve parent Part: {error}"))?.ok_or("code cell parent Part is not published")?;
        if bound.owner != owner || bound.turn_id.as_deref() != Some(turn.turn_id.as_str()) {
            return Err("code cell parent Part ownership mismatch".into());
        }
        store.publish_code_cell(&CodeCellRecord {
            store_epoch: epoch.clone(), conversation_id: conversation.into(), run_id: owner.run_id.clone(),
            parent_part_id: parent.id.clone(), core_revision: state.journal.revision,
            cell: cell.identity().clone(), observation: cell.committed_observation()?,
        }).map_err(|error| format!("publish code cell: store observation: {error}"))?;
    }
    Ok(())
}
