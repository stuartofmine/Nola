// SPDX-License-Identifier: Apache-2.0
//! Transport metadata for the exact trusted destination context.
use nola_domain::generative_ui::{UiError, UiSurfaceOwner, UiSurfacePhase, UiSurfaceState, A2UI_WIRE_VERSION, UI_MAX_ACTION_BYTES};
use serde_json::{json, Map, Value};

/// The caller supplies authoritative state and the admitted destination, never
/// a renderer-provided owner. Preserve complete data models as required by
/// 0.9.1; do not silently trim fields or merge colliding surface names.
pub fn client_data_model(
    destination: &UiSurfaceOwner,
    surfaces: &[UiSurfaceState],
) -> Result<Option<Value>, UiError> {
    destination.validate()?;
    let mut models = Map::new();
    for surface in surfaces {
        let owner = &surface.identity.owner;
        if owner.authority_id != destination.authority_id
            || owner.conversation_id != destination.conversation_id
            || owner.participant_id != destination.participant_id
            || owner.context_id != destination.context_id
            || !surface.send_data_model
            || matches!(surface.phase, UiSurfacePhase::Deleted | UiSurfacePhase::ReadOnly)
        {
            continue;
        }
        surface.identity.validate()?;
        if !surface.data.is_object() {
            return Err(UiError::InvalidMessage);
        }
        if models.insert(surface.identity.surface_id.clone(), surface.data.clone()).is_some() {
            return Err(UiError::InvalidIdentity);
        }
    }
    if models.is_empty() { return Ok(None); }
    let metadata = json!({"version": A2UI_WIRE_VERSION, "surfaces": models});
    if serde_json::to_vec(&metadata).map_err(|_| UiError::InvalidMessage)?.len() > UI_MAX_ACTION_BYTES {
        return Err(UiError::ResourceLimit);
    }
    Ok(Some(metadata))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::generative_ui::UiSurfaceIdentity;
    use std::collections::BTreeMap;

    fn surface() -> UiSurfaceState {
        UiSurfaceState {
            identity: UiSurfaceIdentity {
                owner: UiSurfaceOwner {
                    authority_id: "host".into(), conversation_id: "conversation".into(),
                    participant_id: "agent".into(), context_id: "context".into(),
                    originating_run_id: "run".into(), message_id: "message".into(), part_id: "part".into(),
                },
                surface_id: "form".into(), generation: 1,
            },
            revision: 1, catalog_id: "catalog".into(), catalog_digest: "digest".into(),
            phase: UiSurfacePhase::Ready, components: BTreeMap::new(),
            data: json!({"answer": 42}), theme: None, send_data_model: true,
        }
    }

    #[test]
    fn metadata_is_complete_and_scoped_to_the_destination() {
        let state = surface();
        let owner = state.identity.owner.clone();
        let mut foreign = state.clone();
        foreign.identity.owner.participant_id = "other-agent".into();
        foreign.data = json!({"private": true});
        let result = client_data_model(&owner, &[state.clone(), foreign]).unwrap().unwrap();
        assert_eq!(result, json!({"version": A2UI_WIRE_VERSION, "surfaces": {"form": {"answer": 42}}}));
        for field in ["authority", "conversation", "context"] {
            let mut destination = owner.clone();
            match field {
                "authority" => destination.authority_id = "other".into(),
                "conversation" => destination.conversation_id = "other".into(),
                _ => destination.context_id = "other".into(),
            }
            assert_eq!(client_data_model(&destination, &[state.clone()]).unwrap(), None);
        }
    }

    #[test]
    fn opt_out_closed_ambiguous_or_unrepresentable_models_are_not_sent() {
        let state = surface();
        let owner = state.identity.owner.clone();
        for phase in [UiSurfacePhase::Deleted, UiSurfacePhase::ReadOnly] {
            let mut closed = state.clone(); closed.phase = phase;
            assert_eq!(client_data_model(&owner, &[closed]).unwrap(), None);
        }
        let mut opted_out = state.clone(); opted_out.send_data_model = false;
        assert_eq!(client_data_model(&owner, &[opted_out]).unwrap(), None);
        assert!(client_data_model(&owner, &[state.clone(), state.clone()]).is_err());
        let mut scalar = state.clone(); scalar.data = json!(42);
        assert!(client_data_model(&owner, &[scalar]).is_err());
        let mut oversized = state; oversized.data = json!({"answer": "x".repeat(UI_MAX_ACTION_BYTES)});
        assert!(matches!(client_data_model(&owner, &[oversized]), Err(UiError::ResourceLimit)));
    }
}
