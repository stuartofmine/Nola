// SPDX-License-Identifier: Apache-2.0
//! A2UI application use cases reuse the canonical receipt transaction and journal.
use nola_domain::generative_ui::{UiSnapshot, UiSurfaceIdentity};
use nola_kernel::generative_ui::{projection::project_surface, state::transition_messages};
use nola_store_port::{generative_ui::GenerativeUiRepository, protocol_journal::{AppendProjectionEventInput, CommandProtocolRepositoryProvider, ProjectionJournalRepository, ProjectionScopeKey}};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

fn ui_protocol_failure(error: nola_domain::generative_ui::UiError) -> String {
    use nola_domain::generative_ui::UiError;
    match error {
        UiError::UnsupportedProtocol | UiError::UnsupportedCatalog | UiError::InvalidMessage => format!(
            "{error}. render_ui accepts only v0.9.1: each messages element must include version=\"v0.9.1\" and exactly one message payload. createSurface requires surfaceId and catalogId=\"{}\"; updateComponents and updateDataModel keep surfaceId inside their payload. Button requires child referencing a separate Text node, not label. The entire rejected batch committed nothing; retry corrected arguments with the same expected_revision. Minimal valid arguments: {}",
            nola_kernel::generative_ui::schema::NOLA_CATALOG_ID,
            nola_kernel::generative_ui::schema::minimal_form_example()),
        _ => error.to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublishUiMessages {
    /// Trusted Host execution supplies identity; it is never copied from LLM arguments.
    pub identity: UiSurfaceIdentity,
    pub input: nola_domain::generative_ui_tool::UiToolInput,
    pub occurred_at: String,
}

pub(crate) fn publish_ui_messages<R>(store: &mut R, input: PublishUiMessages) -> Result<UiSnapshot, String>
where R: GenerativeUiRepository + ProjectionJournalRepository + CommandProtocolRepositoryProvider {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let previous = store.read_ui_surface(&input.identity).map_err(|error| error.to_string())?;
        let next = transition_messages(previous.as_ref(), &input.identity, &input.input)
            .map_err(ui_protocol_failure)?;
        // Projection admission is part of the write: unsupported bindings or
        // functions cannot commit a state that no renderer can consume.
        let projected = project_surface(&next).map_err(|error| error.to_string())?;
        if let Some(previous) = previous.as_ref() {
            let mut candidate = next.clone();
            candidate.revision = previous.revision;
            if candidate == *previous {
                // Validation and the expected-revision fence still apply. An
                // unchanged publication is not another Surface revision, part
                // update or renderer event; retain the original history.
                return project_surface(previous).map(|projection| projection.snapshot)
                    .map_err(|error| error.to_string());
            }
        }
        let scope = ProjectionScopeKey { scope_type: "conversation".into(), scope_id: input.identity.owner.conversation_id.clone() };
        let sequence = store.projection_cursor(&scope).map_err(|error| error.to_string())?.map_or(0, |cursor| cursor.current_sequence);
        store.write_ui_surface(input.input.expected_revision, &next).map_err(|error| error.to_string())?;
        store.write_ui_surface_part(&projected.snapshot, input.occurred_at.parse::<i64>().map_err(|_| "invalid UI publication timestamp")?).map_err(|error| error.to_string())?;
        // Stable authority identity + revision, rather than a transport retry ID.
        let identity_bytes = serde_json::to_vec(&input.identity).map_err(|error| error.to_string())?;
        let event_id = format!("ui:{:x}:{}", Sha256::digest(identity_bytes), next.revision);
        store.append_projection_event(&AppendProjectionEventInput {
            event_id, scope, expected_after_sequence: sequence,
            payload_json: serde_json::to_string(&nola_domain::generative_ui::UiSurfaceJournalEvent::changed(&next)
                .map_err(|error| error.to_string())?).map_err(|error| error.to_string())?,
            occurred_at: input.occurred_at,
        }).map_err(|error| error.to_string())?;
        Ok(projected.snapshot)
    })
}

pub fn read_ui_snapshot<R: GenerativeUiRepository>(store: &R, identity: &UiSurfaceIdentity) -> Result<Option<UiSnapshot>, String> {
    identity.validate().map_err(|error| error.to_string())?;
    if identity.owner.authority_id != store.ui_authority_id().map_err(|error| error.to_string())? {
        return Err("UI projection belongs to another authority".into());
    }
    store.read_ui_surface(identity).map_err(|error| error.to_string())?
        .map(|state| project_surface(&state).map(|projection| projection.snapshot).map_err(|error| error.to_string()))
        .transpose()
}

/// Reuse the exact submit edit evaluator without committing its candidate.
/// Checks may fail in a preview; their errors remain in the projected nodes.
pub fn preview_ui_surface<R: GenerativeUiRepository>(store: &R,
    input: nola_domain::generative_ui::UiDraftPreviewInput,
) -> Result<nola_domain::generative_ui::UiDraftPreview, String> {
    use nola_domain::generative_ui::*;
    input.identity.validate().map_err(|e| e.to_string())?;
    if input.draft_generation == 0 || input.draft_generation > 9_007_199_254_740_991
        || input.identity.owner.authority_id != store.ui_authority_id().map_err(|e| e.to_string())? {
        return Err("invalid UI draft preview owner or generation".into());
    }
    let state = store.read_ui_surface(&input.identity).map_err(|e| e.to_string())?
        .ok_or("UI draft preview surface is absent")?;
    let prepared = nola_kernel::generative_ui::interaction::prepare_interaction(&state, &UiInteraction {
        identity: input.identity.clone(), expected_surface_revision: input.expected_surface_revision,
        edits: input.edits, action: None,
    }).map_err(|e| e.to_string())?;
    let mut snapshot = prepared.snapshot;
    snapshot.revision = state.revision;
    Ok(UiDraftPreview { identity: input.identity, base_revision: state.revision,
        draft_generation: input.draft_generation, snapshot })
}

/// Historical display never changes stored phase and exposes no interaction handles.
pub fn read_ui_revision_snapshot<R: GenerativeUiRepository>(store: &R, identity: &UiSurfaceIdentity, revision: u64) -> Result<Option<UiSnapshot>, String> {
    nola_domain::generative_ui::UiSurfaceRevisionRef { identity: identity.clone(), revision }
        .validate().map_err(|error| error.to_string())?;
    if identity.owner.authority_id != store.ui_authority_id().map_err(|error| error.to_string())? {
        return Err("UI projection belongs to another authority".into());
    }
    store.read_ui_surface_revision(identity, revision).map_err(|error| error.to_string())?
        .map(|state| nola_kernel::generative_ui::projection::project_historical_surface(&state)
            .map_err(|error| error.to_string())).transpose()
}

/// Resolve an exact client baseline against immutable versions. No client-side
/// reducer runs A2UI rules; clients only apply the resulting node projection.
pub fn read_ui_surface_changes<R: GenerativeUiRepository>(store: &R, identity: &UiSurfaceIdentity, base_revision: Option<u64>)
    -> Result<nola_domain::generative_ui::UiSurfaceUpdate, String> {
    use nola_domain::generative_ui::{UiSurfaceRevisionRef, UiSurfaceUpdate};
    identity.validate().map_err(|error| error.to_string())?;
    if let Some(revision) = base_revision {
        UiSurfaceRevisionRef { identity: identity.clone(), revision }.validate().map_err(|error| error.to_string())?;
    }
    if identity.owner.authority_id != store.ui_authority_id().map_err(|error| error.to_string())? {
        return Err("UI projection belongs to another authority".into());
    }
    let Some(current) = store.read_ui_surface(identity).map_err(|error| error.to_string())? else {
        return Ok(UiSurfaceUpdate::Missing { identity: identity.clone() });
    };
    if base_revision.is_some_and(|revision| revision > current.revision) {
        return Err("UI projection baseline is ahead of its authority".into());
    }
    if base_revision == Some(current.revision) {
        return Ok(UiSurfaceUpdate::Unchanged { identity: identity.clone(), revision: current.revision });
    }
    let snapshot = project_surface(&current).map_err(|error| error.to_string())?.snapshot;
    let Some(base) = base_revision else { return Ok(UiSurfaceUpdate::Snapshot { snapshot }); };
    let Some(previous) = store.read_ui_surface_revision(identity, base).map_err(|error| error.to_string())? else {
        return Ok(UiSurfaceUpdate::Snapshot { snapshot });
    };
    if previous.catalog_digest != current.catalog_digest || previous.catalog_id != current.catalog_id {
        return Ok(UiSurfaceUpdate::Snapshot { snapshot });
    }
    let previous = project_surface(&previous).map_err(|error| error.to_string())?.snapshot;
    let changes = nola_kernel::generative_ui::projection::changes(&previous, &snapshot).map_err(|error| error.to_string())?;
    // A heavily changed tree can be cheaper to replace than to patch. Both
    // alternatives are authoritative Core projections and have the same owner.
    let delta = UiSurfaceUpdate::Changes { changes };
    let full = UiSurfaceUpdate::Snapshot { snapshot };
    if serde_json::to_vec(&delta).map_err(|error| error.to_string())?.len()
        < serde_json::to_vec(&full).map_err(|error| error.to_string())?.len() { Ok(delta) } else { Ok(full) }
}

pub fn list_ui_snapshots<R: GenerativeUiRepository>(store: &R, conversation_id: &str) -> Result<Vec<UiSnapshot>, String> {
    let authority_id = store.ui_authority_id().map_err(|error| error.to_string())?;
    store.list_ui_surfaces(&authority_id, conversation_id).map_err(|error| error.to_string())?
        .iter().map(|state| project_surface(state).map(|projection| projection.snapshot).map_err(|error| error.to_string())).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteUiTool {
    pub effect_id: String,
    pub request_id: String,
    pub delivery: nola_domain::host_effect::EffectDeliveryLease,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiToolPublication {
    pub identity: UiSurfaceIdentity,
    pub revision: u64,
    /// Whether this invocation committed a new Surface state/revision.
    pub changed: bool,
    pub phase: nola_domain::generative_ui::UiSurfacePhase,
    /// Core-generated readable content for text-only clients and model context.
    pub summary: String,
    /// Guidance describes the committed UI state; it never completes a Run
    /// or fabricates a user interaction.
    pub publication_guidance: String,
}

fn publication_guidance(snapshot: &UiSnapshot) -> String {
    use nola_domain::generative_ui::UiSurfacePhase;
    match snapshot.phase {
        UiSurfacePhase::Ready => "Publication succeeded: the complete UI is committed and available to the client. Do not call render_ui again to display or confirm the same content. If this publication satisfies the requested form/card creation or update, answer briefly and end this response. Otherwise continue only the remaining requested work; do not repeat an already committed step. Further user interaction will arrive as a separate real ui_action input. This receipt is not a new user interaction and does not claim completion of business operations behind the form.".into(),
        UiSurfacePhase::Pending => "Publication committed, but the UI is incomplete. Publish only the missing components/data needed for a valid root; do not repeat unchanged content.".into(),
        UiSurfacePhase::Deleted => "Deletion committed. No additional publication is needed to confirm deletion.".into(),
        UiSurfacePhase::ReadOnly => "The UI is read-only. Do not attempt interactive updates or infer user approval.".into(),
    }
}

/// Consumes the committed ExecuteTool payload; no UI arguments or owner fields
/// are accepted from the invoking Host beyond its existing effect fence.
pub fn execute_ui_tool<R>(store: &mut R, command: ExecuteUiTool) -> Result<UiToolPublication, String>
where R: GenerativeUiRepository + ProjectionJournalRepository + CommandProtocolRepositoryProvider
    + nola_store_port::run_effect::RunEffectRepository
    + nola_store_port::effect_delivery::EffectDeliveryRepository {
    use nola_agent_engine::agent::host::{HostRequest, HostRequestEnvelope};
    use nola_domain::generative_ui_tool::{UiToolInput, UI_TOOL_ID};
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let stored = store.outbox_effect(&command.effect_id).map_err(|error| error.to_string())?
            .ok_or("UI tool effect is not committed")?;
        if stored.request_id != command.request_id { return Err("UI tool request identity mismatch".into()); }
        let request: HostRequestEnvelope = serde_json::from_str(&stored.payload_json).map_err(|error| error.to_string())?;
        let HostRequest::ExecuteTool { capability_id, call_id, input, .. } = &request.payload else {
            return Err("UI publication requires ExecuteTool".into());
        };
        if capability_id != UI_TOOL_ID { return Err("effect does not authorize UI publication".into()); }
        let input: UiToolInput = serde_json::from_value(input.clone()).map_err(|error| error.to_string())?;
        input.validate().map_err(|error| error.to_string())?;
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&input).map_err(|error| error.to_string())?));
        if let Some((previous_digest, result)) = store.ui_tool_publication(&command.effect_id).map_err(|error| error.to_string())? {
            if previous_digest != digest { return Err("UI effect content changed after publication".into()); }
            return serde_json::from_str(&result).map_err(|error| error.to_string());
        }
        if stored.status != "pending" { return Err("UI effect is no longer pending".into()); }
        let attempt = store.active_effect_delivery_attempt(&command.effect_id).map_err(|error| error.to_string())?
            .ok_or("UI effect has no active delivery")?;
        let lease = &command.delivery;
        if attempt.id != lease.attempt_id || attempt.run_id != stored.run_id
            || attempt.request_id != command.request_id
            || attempt.state != nola_domain::EffectDeliveryAttemptState::ExecutionStarted
            || attempt.worker_generation.as_deref() != Some(lease.worker_generation.as_str())
            || attempt.lease_owner.as_deref() != Some(lease.lease_owner.as_str())
            || attempt.lease_token.as_deref() != Some(lease.lease_token.as_str())
            || attempt.lease_expires_at_ms.is_none_or(|expiry| expiry <= command.now_ms)
        { return Err("UI effect delivery fence is stale".into()); }
        let turn_id = request.turn_id.as_deref().ok_or("UI effect has no Turn")?;
        let owner = store.ui_tool_owner(&stored.run_id, turn_id, call_id).map_err(|error| error.to_string())?;
        let first = input.messages.first().ok_or("UI message batch is empty")?;
        let first = nola_kernel::generative_ui::parse_a2ui_message(&serde_json::to_vec(first).map_err(|error| error.to_string())?)
            .map_err(ui_protocol_failure)?;
        let surface_id = first.surface_id().to_owned();
        let existing = store.list_ui_surfaces(&owner.authority_id, &owner.conversation_id).map_err(|error| error.to_string())?
            .into_iter().filter(|surface| surface.identity.owner.context_id == owner.context_id
                && surface.identity.owner.participant_id == owner.participant_id && surface.identity.surface_id == surface_id
                // Tombstones retain immutable history, but no longer occupy
                // the live name. A new create receives a fresh generation.
                && surface.phase != nola_domain::generative_ui::UiSurfacePhase::Deleted)
            .collect::<Vec<_>>();
        if existing.len() > 1 { return Err("UI surface name is ambiguous in this context".into()); }
        let identity = match (existing.first(), &first) {
            (None, nola_kernel::generative_ui::A2uiMessage::Create(_)) => {
                let generation = store.next_ui_generation(&owner, &surface_id).map_err(|error| error.to_string())?;
                UiSurfaceIdentity { owner, surface_id, generation }
            },
            (Some(_), nola_kernel::generative_ui::A2uiMessage::Create(_)) => return Err("UI surface already exists".into()),
            (Some(surface), _) => surface.identity.clone(),
            _ => return Err("UI surface does not exist in this context".into()),
        };
        if let Some(current) = existing.first() {
            if input.expected_revision != current.revision {
                let snapshot = project_surface(current).map_err(|error| error.to_string())?.snapshot;
                return Err(json!({
                    "error":"ui_revision_conflict",
                    "surface":current.identity,
                    "expected_revision":input.expected_revision,
                    "current_revision":current.revision,
                    "current_summary":snapshot.summary,
                    "guidance":"The surface changed after your last publication, possibly through a user edit. Nothing in this batch was committed. Review the current user interaction before retrying; preserve unrelated user data. If your intended patch still applies, submit a new render_ui call with current_revision as expected_revision. Do not guess or automatically overwrite user values."
                }).to_string());
            }
        }
        let previous_revision = input.expected_revision;
        let snapshot = publish_ui_messages(store, PublishUiMessages { identity, input, occurred_at: command.now_ms.to_string() })?;
        let changed = snapshot.revision != previous_revision;
        let publication_guidance = if changed { publication_guidance(&snapshot) } else {
            format!("No change: this exact UI content is already committed at revision {}. No new revision or renderer update was emitted. {}",
                snapshot.revision, publication_guidance(&snapshot))
        };
        let result = UiToolPublication { identity: snapshot.identity, revision: snapshot.revision,
            changed, publication_guidance,
            phase: snapshot.phase, summary: snapshot.summary };
        store.record_ui_tool_publication(&command.effect_id, &digest, &serde_json::to_string(&result).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        Ok(result)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitUiInteraction {
    pub interaction: nola_domain::generative_ui::UiInteraction,
    pub submission_id: String,
    pub submitted_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiInteractionReceipt {
    pub snapshot: UiSnapshot,
    pub queued_submission_id: Option<String>,
}

/// Add authoritative transport data to the existing context pipeline. It is
/// data, not a system instruction or a second client-specific memory store.
pub(crate) fn attach_client_data_model(
    submission: &mut nola_agent_engine::agent::AgentSubmission,
    surfaces: &[nola_domain::generative_ui::UiSurfaceState],
) -> Result<(), String> {
    use nola_agent_engine::agent::AgentSubmission;
    use nola_agent_engine::context_engine::{ContextComponent, ContextComponentKind, ContextComponentSource, ContextPlatform};
    const COMPONENT_ID: &str = "nola:a2ui:client-data-model";
    let components = match submission {
        AgentSubmission::UserInput { turn_context_components, .. }
        | AgentSubmission::GoalUserInput { turn_context_components, .. }
        | AgentSubmission::PromoteQueuedInput { turn_context_components, .. } => turn_context_components,
        _ => return Ok(()),
    };
    if components.iter().any(|component| component.id == COMPONENT_ID || component.kind == ContextComponentKind::UiClientData) {
        return Err("A2UI client data metadata is owned by Core".into());
    }
    let Some(first) = surfaces.first() else { return Ok(()); };
    let Some(data_model) = nola_kernel::generative_ui::metadata::client_data_model(
        &first.identity.owner, surfaces).map_err(|error| error.to_string())? else { return Ok(()); };
    components.push(ContextComponent {
        kind: ContextComponentKind::UiClientData,
        id: COMPONENT_ID.into(),
        content: json!({"metadata": {"a2uiClientDataModel": data_model}}),
        source: Some(ContextComponentSource {
            platform: Some(ContextPlatform::Shared), title: Some("Current UI form data".into()),
            mime_type: Some("application/json".into()), ..Default::default()
        }),
    });
    Ok(())
}

#[cfg(test)]
mod client_metadata_tests {
    use super::*;
    use nola_agent_engine::agent::AgentSubmission;
    use nola_domain::generative_ui::*;

    #[test]
    fn ordinary_query_keeps_user_text_and_gets_core_owned_form_context() {
        let mut input = AgentSubmission::UserInput {
            items: vec![nola_agent_engine::agent::AgentItem::user_message(
                nola_agent_engine::agent::ItemId::from_string("user-query".into()), "Use my updated form")],
            turn_context_items: vec![], turn_context_platform: Default::default(),
            turn_context_components: vec![], submission_id: Some("query".into()),
            submitted_at_epoch_ms: Some(1), delivery: nola_agent_engine::agent::AgentInputDelivery::NextTurn,
        };
        let state = UiSurfaceState {
            identity: UiSurfaceIdentity {
                owner: UiSurfaceOwner {
                    authority_id: "host".into(), conversation_id: "conversation".into(),
                    participant_id: "agent".into(), context_id: "context".into(),
                    originating_run_id: "run".into(), message_id: "message".into(), part_id: "part".into(),
                }, surface_id: "form".into(), generation: 1,
            }, revision: 2, catalog_id: "catalog".into(), catalog_digest: "digest".into(),
            phase: UiSurfacePhase::Ready, components: Default::default(),
            data: json!({"answer":"updated"}), theme: None, send_data_model: true,
        };
        let before_items = match &input { AgentSubmission::UserInput { items, .. } => serde_json::to_value(items).unwrap(), _ => unreachable!() };
        attach_client_data_model(&mut input, &[state.clone()]).unwrap();
        let AgentSubmission::UserInput { items, turn_context_components, submission_id, .. } = &input else { unreachable!() };
        assert_eq!(serde_json::to_value(items).unwrap(), before_items);
        assert_eq!(submission_id.as_deref(), Some("query"));
        assert_eq!(turn_context_components.len(), 1);
        assert_eq!(turn_context_components[0].content["metadata"]["a2uiClientDataModel"],
            json!({"version":"v0.9.1","surfaces":{"form":{"answer":"updated"}}}));
        // A caller cannot replay or substitute this reserved component, even
        // when there are no current Surfaces to overwrite the forged value.
        assert!(attach_client_data_model(&mut input, &[]).is_err());
        let mut successor = AgentSubmission::PromoteQueuedInput {
            admission_id: "queued".into(), expected_previous_turn_id: "previous".into(),
            successor_turn_id: "next".into(), turn_context_components: Vec::new(),
        };
        attach_client_data_model(&mut successor, &[state]).unwrap();
        let AgentSubmission::PromoteQueuedInput { turn_context_components, .. } = successor else { unreachable!() };
        assert_eq!(turn_context_components[0].content["metadata"]["a2uiClientDataModel"]["surfaces"]["form"]["answer"], "updated");
    }
}

fn ui_action_summary(envelope: &serde_json::Value) -> String {
    let name = envelope.pointer("/message/action/name").and_then(serde_json::Value::as_str).unwrap_or("");
    let name: String = name.chars().filter(|character| !character.is_control()).take(120).collect();
    if name.trim().is_empty() { "Submitted form".into() } else { format!("Submitted form · {}", name.trim()) }
}

pub fn submit_ui_interaction<R>(store: &mut R, input: SubmitUiInteraction) -> Result<UiInteractionReceipt, String>
where R: GenerativeUiRepository + ProjectionJournalRepository + CommandProtocolRepositoryProvider
    + nola_store_port::agent_queue::AgentQueueRepository {
    use nola_agent_engine::agent::{AgentSubmission, AgentItem, AgentInputDelivery, ItemId};
    use nola_agent_engine::agent::host::AgentBoundaryInput;
    use nola_store_port::agent_queue::EnqueueAgentCommandInput;
    nola_domain::generative_ui::validate_ui_submission_id(&input.submission_id)
        .map_err(|error| error.to_string())?;
    if input.submitted_at_ms < 0 {
        return Err("invalid UI submission identity".into());
    }
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(input.submitted_at_ms)
        .ok_or("UI submission timestamp is outside the supported range")?
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let original = store.read_ui_surface(&input.interaction.identity).map_err(|error| error.to_string())?
            .ok_or("UI surface no longer exists")?;
        let prepared = nola_kernel::generative_ui::interaction::prepare_interaction(&original, &input.interaction)
            .map_err(|error| error.to_string())?;
        let scope = ProjectionScopeKey { scope_type: "conversation".into(), scope_id: original.identity.owner.conversation_id.clone() };
        let sequence = store.projection_cursor(&scope).map_err(|error| error.to_string())?.map_or(0, |cursor| cursor.current_sequence);
        store.write_ui_surface(original.revision, &prepared.state).map_err(|error| error.to_string())?;
        store.write_ui_surface_part(&prepared.snapshot, input.submitted_at_ms).map_err(|error| error.to_string())?;
        let queued = if let Some(mut event) = prepared.event {
            // Core materializes an explicit user interaction; clients do not
            // rewrite prompts or embed instructions from display text.
            event["timestamp"] = serde_json::Value::String(timestamp.clone());
            let message = json!({"version":nola_domain::generative_ui::A2UI_WIRE_VERSION,"action":event});
            nola_kernel::generative_ui::schema::validate_client_message(&message).map_err(|error| error.to_string())?;
            let states = store.list_ui_surfaces(&original.identity.owner.authority_id,
                &original.identity.owner.conversation_id).map_err(|error| error.to_string())?;
            let data_model = nola_kernel::generative_ui::metadata::client_data_model(
                &original.identity.owner, &states).map_err(|error| error.to_string())?;
            let mut envelope = json!({"type":"ui_action","surface":original.identity,"message":message});
            if let Some(data_model) = data_model {
                envelope["metadata"] = json!({"a2uiClientDataModel": data_model});
            }
            let content = envelope.to_string();
            // Bound the complete queued envelope, not just the client event.
            // Failure rolls back the Surface edit and its publication as well.
            if content.len() > nola_domain::generative_ui::UI_MAX_ACTION_BYTES {
                return Err("UI action envelope exceeds the byte budget".into());
            }
            let submission = AgentSubmission::UserInput {
                items: vec![AgentItem::UiAction {
                    id: ItemId::from_string(format!("ui:{}:user", input.submission_id)),
                    action: nola_domain::generative_ui::UiActionContent {
                        surface: nola_domain::generative_ui::UiSurfaceRevisionRef {
                            identity: original.identity.clone(), revision: prepared.snapshot.revision,
                        },
                        summary: ui_action_summary(&envelope),
                        envelope,
                    },
                }],
                turn_context_items: Vec::new(), turn_context_platform: Default::default(), turn_context_components: Vec::new(),
                submission_id: Some(input.submission_id.clone()), submitted_at_epoch_ms: Some(input.submitted_at_ms),
                delivery: AgentInputDelivery::NextTurn,
            };
            let payload_json = serde_json::to_string(&AgentBoundaryInput::Submission { submission }).map_err(|error| error.to_string())?;
            store.enqueue_agent_command(&EnqueueAgentCommandInput {
                session_id: original.identity.owner.context_id.clone(), command_id: input.submission_id.clone(),
                payload_json, created_at_ms: input.submitted_at_ms,
            }).map_err(|error| error.to_string())?;
            Some(input.submission_id.clone())
        } else { None };
        let identity_bytes = serde_json::to_vec(&original.identity).map_err(|error| error.to_string())?;
        store.append_projection_event(&AppendProjectionEventInput {
            event_id: format!("ui:{:x}:{}", Sha256::digest(identity_bytes), prepared.state.revision), scope,
            expected_after_sequence: sequence,
            payload_json: serde_json::to_string(&nola_domain::generative_ui::UiSurfaceJournalEvent::changed(&prepared.state)
                .map_err(|error| error.to_string())?).map_err(|error| error.to_string())?,
            occurred_at: input.submitted_at_ms.to_string(),
        }).map_err(|error| error.to_string())?;
        Ok(UiInteractionReceipt { snapshot: prepared.snapshot, queued_submission_id: queued })
    })
}

#[cfg(test)]
mod projection_changes_tests {
    use super::*;
    use nola_domain::generative_ui::*;

    struct Versions(Vec<UiSurfaceState>);
    impl GenerativeUiRepository for Versions {
        type Error = String;
        fn ui_authority_id(&self) -> Result<String, String> { Ok("host".into()) }
        fn read_ui_surface(&self, id: &UiSurfaceIdentity) -> Result<Option<UiSurfaceState>, String> {
            Ok(self.0.iter().filter(|state| &state.identity == id).max_by_key(|state| state.revision).cloned())
        }
        fn read_ui_surface_revision(&self, id: &UiSurfaceIdentity, revision: u64) -> Result<Option<UiSurfaceState>, String> {
            Ok(self.0.iter().find(|state| &state.identity == id && state.revision == revision).cloned())
        }
        fn ui_tool_owner(&self, _: &str, _: &str, _: &str) -> Result<UiSurfaceOwner, String> { unreachable!() }
        fn next_ui_generation(&self, _: &UiSurfaceOwner, _: &str) -> Result<u64, String> { unreachable!() }
        fn ui_tool_publication(&self, _: &str) -> Result<Option<(String, String)>, String> { unreachable!() }
        fn record_ui_tool_publication(&mut self, _: &str, _: &str, _: &str) -> Result<(), String> { unreachable!() }
        fn write_ui_surface(&mut self, _: u64, _: &UiSurfaceState) -> Result<(), String> { unreachable!() }
        fn write_ui_surface_part(&mut self, _: &UiSnapshot, _: i64) -> Result<(), String> { unreachable!() }
        fn list_ui_surfaces(&self, _: &str, _: &str) -> Result<Vec<UiSurfaceState>, String> { unreachable!() }
    }
    fn fixture() -> UiSurfaceState {
        let children: Vec<_> = (0..20).map(|n| format!("text-{n}")).collect();
        let mut components = std::collections::BTreeMap::new();
        components.insert("root".into(), json!({"id":"root","component":"Column","children":children}));
        for id in children { components.insert(id.clone(), json!({"id":id,"component":"Text","text":"stable label"})); }
        UiSurfaceState {
            identity: UiSurfaceIdentity { owner: UiSurfaceOwner { authority_id: "host".into(), conversation_id: "chat".into(),
                participant_id: "assistant".into(), context_id: "chat".into(), originating_run_id: "run".into(),
                message_id: "message".into(), part_id: "tool".into() }, surface_id: "form".into(), generation: 1 },
            revision: 1, catalog_id: nola_kernel::generative_ui::schema::NOLA_CATALOG_ID.into(),
            catalog_digest: nola_kernel::generative_ui::schema::catalog_digest(), phase: UiSurfacePhase::Ready,
            components, data: json!({}), theme: None, send_data_model: false,
        }
    }
    #[test]
    fn one_node_change_is_a_patch_and_reconstructs_the_authoritative_projection() {
        let original = fixture();
        let mut next = original.clone();
        next.revision = 2;
        next.components.get_mut("text-3").unwrap()["text"] = json!("updated");
        let versions = Versions(vec![original.clone(), next.clone()]);
        let UiSurfaceUpdate::Changes { changes } = read_ui_surface_changes(&versions, &next.identity, Some(1)).unwrap() else { panic!("expected bounded node delta") };
        assert_eq!(changes.base_revision, 1);
        assert_eq!(changes.revision, 2);
        assert_eq!(changes.upserted_nodes.len(), 1);
        let mut reconstructed = project_surface(&original).unwrap().snapshot;
        for id in changes.removed_instance_ids { reconstructed.nodes.remove(&id); }
        for node in changes.upserted_nodes { reconstructed.nodes.insert(node.instance_id.clone(), node); }
        reconstructed.revision = changes.revision;
        reconstructed.root_instance_id = changes.root_instance_id;
        reconstructed.phase = changes.phase;
        reconstructed.summary = changes.summary;
        assert_eq!(reconstructed, project_surface(&next).unwrap().snapshot);
    }
    #[test]
    fn missing_baseline_resynchronizes_but_future_revision_and_foreign_authority_are_rejected() {
        let mut state = fixture();
        state.revision = 3;
        let versions = Versions(vec![state.clone()]);
        assert!(matches!(read_ui_surface_changes(&versions, &state.identity, Some(1)).unwrap(), UiSurfaceUpdate::Snapshot { .. }));
        assert!(matches!(read_ui_surface_changes(&versions, &state.identity, None).unwrap(), UiSurfaceUpdate::Snapshot { .. }));
        assert!(matches!(read_ui_surface_changes(&versions, &state.identity, Some(3)).unwrap(), UiSurfaceUpdate::Unchanged { revision: 3, .. }));
        assert!(read_ui_surface_changes(&versions, &state.identity, Some(4)).is_err());
        assert!(read_ui_surface_changes(&versions, &state.identity, Some(0)).is_err());
        let mut other = state.identity.clone();
        other.generation += 1;
        assert!(matches!(read_ui_surface_changes(&versions, &other, None).unwrap(), UiSurfaceUpdate::Missing { .. }));
        other.owner.authority_id = "other-host".into();
        assert!(read_ui_surface_changes(&versions, &other, None).is_err());
        assert!(read_ui_snapshot(&versions, &other).is_err());
        assert!(read_ui_revision_snapshot(&versions, &other, 1).is_err());
    }
    #[test]
    fn deletion_removes_every_rendered_node_without_crossing_surface_generations() {
        let original = fixture();
        let mut deleted = original.clone();
        deleted.revision = 2;
        deleted.phase = UiSurfacePhase::Deleted;
        let versions = Versions(vec![original, deleted.clone()]);
        match read_ui_surface_changes(&versions, &deleted.identity, Some(1)).unwrap() {
            UiSurfaceUpdate::Changes { changes } => {
                assert_eq!(changes.phase, UiSurfacePhase::Deleted);
                assert!(changes.upserted_nodes.is_empty());
                assert!(!changes.removed_instance_ids.is_empty());
                assert!(changes.root_instance_id.is_none());
            }
            UiSurfaceUpdate::Snapshot { snapshot } => {
                assert_eq!(snapshot.phase, UiSurfacePhase::Deleted);
                assert!(snapshot.nodes.is_empty() && snapshot.root_instance_id.is_none());
            }
            _ => panic!("deletion must materialize the new projection"),
        }
    }
}
